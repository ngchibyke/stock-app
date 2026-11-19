//! Re-polling successful payouts for bank reversals (plan 6, task 8).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};

use server::{
    escrow::{ReleaseReason, jobs::run_due_jobs, release::settle_payout},
    payments::PayoutState,
};
use sqlx::PgPool;
use uuid::Uuid;

struct Paid {
    order_id: Uuid,
    payout_id: Uuid,
    reference: String,
}

/// A delivered order released, its payout accepted and then settled successful.
async fn successful_payout(app: &TestApp) -> Paid {
    let o = app.shipped_order("self", "delivered").await;
    server::escrow::release_order(&app.state, o.order_id, ReleaseReason::BuyerConfirmed)
        .await
        .unwrap();
    let (payout_id, reference): (Uuid, String) =
        sqlx::query_as("SELECT id, reference FROM payouts WHERE order_id = $1")
            .bind(o.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    app.fake.set_payout_state(
        &reference,
        PayoutState::Successful {
            fee_kobo: 3_000,
            amount_kobo: None,
        },
    );
    settle_payout(&app.state, &reference).await.unwrap();
    assert_eq!(status(app, payout_id).await, "successful");
    Paid {
        order_id: o.order_id,
        payout_id,
        reference,
    }
}

async fn status(app: &TestApp, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM payouts WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// Makes the payout look like it was last asked about `hours` ago.
async fn polled_hours_ago(app: &TestApp, id: Uuid, hours: i32) {
    sqlx::query(
        "UPDATE payouts SET last_polled_at = now() - make_interval(hours => $2) WHERE id = $1",
    )
    .bind(id)
    .bind(hours)
    .execute(&app.pool)
    .await
    .unwrap();
}

fn reversed(app: &TestApp, reference: &str) {
    app.fake.set_payout_state(
        reference,
        PayoutState::Reversed {
            reason: "account closed".into(),
        },
    );
}

async fn reversal_postings(app: &TestApp, order_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(DISTINCT txn_id) FROM ledger_entries WHERE order_id = $1 AND kind = 'payout_reversal'",
    )
    .bind(order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn get(app: &TestApp, path: &str, token: &str) -> TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

#[sqlx::test]
async fn the_job_finds_a_reversal_and_raises_an_alert(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let p = successful_payout(&app).await;
    reversed(&app, &p.reference);
    polled_hours_ago(&app, p.payout_id, 7).await;

    run_due_jobs(&app.state).await;

    // Reversed after success: the reverse posting is booked and the payout
    // goes to the owner (`manual`) like any failed one.
    assert_eq!(reversal_postings(&app, p.order_id).await, 1);
    assert_eq!(status(&app, p.payout_id).await, "manual");
    let alerts = get(&app, "/admin/alerts?state=open", &owner).await;
    let kinds: Vec<&str> = alerts.json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec!["payout.reversed_after_success"],
        "{}",
        alerts.json
    );
    assert_eq!(
        alerts.json["items"][0]["subject_id"],
        p.payout_id.to_string()
    );

    // Not twice: another pass finds nothing to book.
    polled_hours_ago(&app, p.payout_id, 7).await;
    run_due_jobs(&app.state).await;
    assert_eq!(reversal_postings(&app, p.order_id).await, 1);
}

#[sqlx::test]
async fn a_successful_payout_is_polled_only_when_due(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let p = successful_payout(&app).await;
    let base = app.fake.calls("payout_status");

    // Just settled: not asked again yet.
    run_due_jobs(&app.state).await;
    assert_eq!(app.fake.calls("payout_status"), base);

    // Six hours later it is, once, and an unchanged answer changes nothing.
    polled_hours_ago(&app, p.payout_id, 7).await;
    run_due_jobs(&app.state).await;
    assert_eq!(app.fake.calls("payout_status"), base + 1);
    assert_eq!(status(&app, p.payout_id).await, "successful");
    run_due_jobs(&app.state).await;
    assert_eq!(app.fake.calls("payout_status"), base + 1, "pushed back");
    assert_eq!(reversal_postings(&app, p.order_id).await, 0);

    // Past the window it is never asked again.
    sqlx::query("UPDATE payouts SET settled_at = now() - interval '20 days' WHERE id = $1")
        .bind(p.payout_id)
        .execute(&pool)
        .await
        .unwrap();
    polled_hours_ago(&app, p.payout_id, 30).await;
    run_due_jobs(&app.state).await;
    assert_eq!(app.fake.calls("payout_status"), base + 1);
}

#[sqlx::test]
async fn an_owner_settled_payout_is_never_repolled(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let p = successful_payout(&app).await;
    sqlx::query("UPDATE payouts SET settled_by = 'owner' WHERE id = $1")
        .bind(p.payout_id)
        .execute(&pool)
        .await
        .unwrap();
    polled_hours_ago(&app, p.payout_id, 30).await;
    let base = app.fake.calls("payout_status");
    run_due_jobs(&app.state).await;
    assert_eq!(app.fake.calls("payout_status"), base);
}

#[sqlx::test]
async fn the_owner_can_repoll_now(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let p = successful_payout(&app).await;
    let path = format!("/admin/payouts/{}/repoll", p.payout_id);

    assert_eq!(
        app.request(Method::POST, &path, None, Some(&support))
            .await
            .status,
        403
    );
    // Unchanged answer: nothing happens.
    let same = app.request(Method::POST, &path, None, Some(&owner)).await;
    assert_eq!(same.status, 200, "{}", same.json);
    assert_eq!(same.json["status"], "successful");

    reversed(&app, &p.reference);
    let res = app.request(Method::POST, &path, None, Some(&owner)).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "failed");
    assert_eq!(res.json["answer"], "settled");
    assert_eq!(reversal_postings(&app, p.order_id).await, 1);

    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_log WHERE action = 'admin.payout_repolled' AND subject_id = $1",
    )
    .bind(p.payout_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 2);

    let missing = format!("/admin/payouts/{}/repoll", Uuid::new_v4());
    assert_eq!(
        app.request(Method::POST, &missing, None, Some(&owner))
            .await
            .status,
        404
    );
}

#[sqlx::test]
async fn a_provider_outage_is_reported_not_swallowed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let p = successful_payout(&app).await;
    app.fake
        .fail_payout_status_with(server::payments::ProviderError::Unavailable(
            anyhow::anyhow!("down"),
        ));
    let res = app
        .request(
            Method::POST,
            &format!("/admin/payouts/{}/repoll", p.payout_id),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 503, "{}", res.json);
    assert_eq!(status(&app, p.payout_id).await, "successful");
}
