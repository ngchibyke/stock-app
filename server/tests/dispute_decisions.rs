//! The owner's decision on a dispute (plan 5, task 5): a full refund, a
//! release or a partial refund, through `POST /admin/disputes/{id}/decide`.
//! The owner's plain release, refund and unhold refuse an order with an open
//! dispute, so every decision goes through the dispute.

mod common;

use axum::http::Method;
use common::{ShippedOrder, TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    escrow::jobs::run_due_jobs,
    money::{self, FeeSchedule},
    payments::ProviderError,
};
use sqlx::PgPool;
use uuid::Uuid;

const COMPLAINT: &str = "The zip was torn and the lining was stained when it arrived";
const REPLY: &str = "It left the shop in perfect condition and was packed in a box";
const NOTE: &str = "Photos confirm the damage";

async fn open_id(app: &TestApp, o: &ShippedOrder) -> String {
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/dispute", o.order_id),
            Some(
                json!({ "reason": "damaged", "body": COMPLAINT, "photo_keys": ["evidence/1.jpg"] }),
            ),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    res.json["id"].as_str().unwrap().to_string()
}

/// A dispute the seller answered: `awaiting_decision`.
async fn answered(app: &TestApp, o: &ShippedOrder) -> String {
    let id = open_id(app, o).await;
    let res = app
        .request(
            Method::POST,
            &format!("/disputes/{id}/response"),
            Some(json!({ "body": REPLY, "photo_keys": [] })),
            Some(&o.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    id
}

async fn decide(app: &TestApp, id: &str, token: &str, body: Value) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/admin/disputes/{id}/decide"),
        Some(body),
        Some(token),
    )
    .await
}

fn code(res: &TestResponse) -> (u16, Option<&str>) {
    (res.status, res.json["error"]["code"].as_str())
}

async fn age_deadline(app: &TestApp, id: &str) {
    sqlx::query(
        "UPDATE disputes SET seller_respond_by = now() - interval '1 minute' WHERE id = $1::uuid",
    )
    .bind(id)
    .execute(&app.pool)
    .await
    .unwrap();
}

async fn text(app: &TestApp, sql: &str, id: Uuid) -> String {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn order_status(app: &TestApp, order: Uuid) -> String {
    text(app, "SELECT status::text FROM orders WHERE id = $1", order).await
}

async fn held(app: &TestApp, order: Uuid) -> bool {
    sqlx::query_scalar("SELECT held_at IS NOT NULL FROM orders WHERE id = $1")
        .bind(order)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn count(app: &TestApp, sql: &str, id: Uuid) -> i64 {
    sqlx::query_scalar(sql)
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn refunds(app: &TestApp, order: Uuid) -> i64 {
    count(
        app,
        "SELECT count(*) FROM refunds WHERE order_id = $1",
        order,
    )
    .await
}

async fn payouts(app: &TestApp, order: Uuid) -> i64 {
    count(
        app,
        "SELECT count(*) FROM payouts WHERE order_id = $1",
        order,
    )
    .await
}

async fn owner_id(app: &TestApp) -> Uuid {
    sqlx::query_scalar("SELECT id FROM users WHERE role = 'owner'")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[derive(Debug, sqlx::FromRow)]
struct DisputeRow {
    status: String,
    decision: Option<String>,
    refund_kobo: Option<i64>,
    decided_by: Option<Uuid>,
    decided: bool,
}

async fn dispute_row(app: &TestApp, id: &str) -> DisputeRow {
    sqlx::query_as(
        "SELECT status, decision, refund_kobo, decided_by, decided_at IS NOT NULL AS decided \
         FROM disputes WHERE id = $1::uuid",
    )
    .bind(id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn decided_audits(app: &TestApp) -> Vec<(Option<Uuid>, Value)> {
    sqlx::query_as(
        "SELECT actor_id, detail FROM audit_log WHERE action = 'admin.dispute_decided' \
         ORDER BY id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn full_refund_decision(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("courier", "delivered").await;
    let id = answered(&app, &o).await;
    app.fake.set_refund_supported(true);
    // The provider answers slowly: the decision is committed before the call,
    // so the order is visibly `refund_pending` while the refund is in flight.
    app.fake
        .delay_next_refund(std::time::Duration::from_millis(800));

    let (res, mid) = tokio::join!(
        decide(
            &app,
            &id,
            &owner,
            json!({ "decision": "full_refund", "note": NOTE })
        ),
        async {
            let mut seen = String::new();
            for _ in 0..60 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                seen = order_status(&app, o.order_id).await;
                if seen != "delivered" {
                    break;
                }
            }
            seen
        }
    );
    assert_eq!(mid, "refund_pending");
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "decided");
    assert_eq!(res.json["decision"]["decision"], "full_refund");
    assert!(res.json["decision"]["refund_kobo"].is_null());

    let (kind, reason, status, amount): (String, String, String, i64) = sqlx::query_as(
        "SELECT kind, reason, status::text, amount_kobo FROM refunds WHERE order_id = $1",
    )
    .bind(o.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        (kind.as_str(), reason.as_str(), status.as_str()),
        ("full", "dispute_refund", "processing")
    );
    let (items, delivery): (i64, i64) =
        sqlx::query_as("SELECT items_total_kobo, delivery_fee_kobo FROM orders WHERE id = $1")
            .bind(o.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(amount, items + delivery);
    assert_eq!(order_status(&app, o.order_id).await, "refunded");
    assert_eq!(payouts(&app, o.order_id).await, 0);

    let d = dispute_row(&app, &id).await;
    assert_eq!(d.status, "decided");
    assert_eq!(d.decision.as_deref(), Some("full_refund"));
    assert_eq!(d.refund_kobo, None);
    assert_eq!(d.decided_by, Some(owner_id(&app).await));
    assert!(d.decided);

    let audits = decided_audits(&app).await;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].0, Some(owner_id(&app).await));
    assert_eq!(
        audits[0].1,
        json!({ "dispute_id": id, "decision": "full_refund", "refund_kobo": null, "note": NOTE })
    );

    // Final: a second decision is refused and changes nothing.
    let again = decide(
        &app,
        &id,
        &owner,
        json!({ "decision": "release", "note": NOTE }),
    )
    .await;
    assert_eq!(code(&again), (409, Some("dispute_closed")));
    assert_eq!(refunds(&app, o.order_id).await, 1);
    assert_eq!(payouts(&app, o.order_id).await, 0);
}

#[sqlx::test]
async fn release_decision(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("courier", "delivered").await;
    let id = answered(&app, &o).await;
    assert!(held(&app, o.order_id).await);

    let res = decide(
        &app,
        &id,
        &owner,
        json!({ "decision": "release", "note": "Seller shipped as described" }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["decision"]["decision"], "release");

    let (status, reason): (String, Option<String>) =
        sqlx::query_as("SELECT status::text, release_reason FROM payouts WHERE order_id = $1")
            .bind(o.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(reason.as_deref(), Some("owner_decision"));
    // The payment is old enough: the payout went out after the commit.
    assert_ne!(status, "scheduled", "payout sent inline");
    assert_eq!(app.fake.payouts().len(), 1);
    assert!(!held(&app, o.order_id).await);
    let hold_reason: Option<String> =
        sqlx::query_scalar("SELECT hold_reason FROM orders WHERE id = $1")
            .bind(o.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(hold_reason, None);
    assert_eq!(order_status(&app, o.order_id).await, "completed");
    assert_eq!(refunds(&app, o.order_id).await, 0);
    assert_eq!(dispute_row(&app, &id).await.status, "decided");
    let audits = decided_audits(&app).await;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].1["decision"], "release");
    assert_eq!(audits[0].1["note"], "Seller shipped as described");
}

#[derive(Debug, sqlx::FromRow)]
struct OrderAmounts {
    items_total_kobo: i64,
    platform_cut_bps: i32,
    platform_cut_kobo: i64,
    collection_fee_share_kobo: i64,
    fee_small_kobo: i64,
    fee_large_kobo: i64,
    fee_threshold_kobo: i64,
    stamp_duty_kobo: i64,
    stamp_duty_threshold_kobo: i64,
}

#[sqlx::test]
async fn partial_decision(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    app.fake.set_refund_supported(true);
    let o = app.shipped_order("courier", "delivered").await;
    let id = answered(&app, &o).await;
    let before: OrderAmounts = sqlx::query_as(
        "SELECT items_total_kobo, platform_cut_bps, platform_cut_kobo, \
                collection_fee_share_kobo, fee_small_kobo, fee_large_kobo, fee_threshold_kobo, \
                stamp_duty_kobo, stamp_duty_threshold_kobo FROM orders WHERE id = $1",
    )
    .bind(o.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let items = before.items_total_kobo;

    // refund_kobo is required for a partial refund, refused otherwise, and
    // must be in 1..items_total.
    for body in [
        json!({ "decision": "partial_refund", "note": NOTE }),
        json!({ "decision": "release", "refund_kobo": 100, "note": NOTE }),
        json!({ "decision": "full_refund", "refund_kobo": 100, "note": NOTE }),
        json!({ "decision": "partial_refund", "refund_kobo": 100 }),
        json!({ "decision": "split", "note": NOTE }),
    ] {
        let r = decide(&app, &id, &owner, body.clone()).await;
        assert_eq!(r.status, 422, "{body}: {}", r.json);
    }
    for bad in [0, -5, items, items + 1] {
        let r = decide(
            &app,
            &id,
            &owner,
            json!({ "decision": "partial_refund", "refund_kobo": bad, "note": NOTE }),
        )
        .await;
        assert_eq!(code(&r), (422, Some("refund_out_of_range")), "{bad}");
    }
    assert_eq!(dispute_row(&app, &id).await.status, "awaiting_decision");
    assert_eq!(refunds(&app, o.order_id).await, 0);

    let refund = 3_000;
    let res = decide(
        &app,
        &id,
        &owner,
        json!({ "decision": "partial_refund", "refund_kobo": refund, "note": NOTE }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["decision"]["decision"], "partial_refund");
    assert_eq!(res.json["decision"]["refund_kobo"], refund);

    // Task 4's amounts: cut on the retained items, the order's own fee shares.
    let retained = items - refund;
    let cut = money::cut_kobo(retained, before.platform_cut_bps);
    let schedule = FeeSchedule {
        small_fee: before.fee_small_kobo,
        large_fee: before.fee_large_kobo,
        threshold: before.fee_threshold_kobo,
        stamp_duty_kobo: before.stamp_duty_kobo,
        stamp_duty_threshold: before.stamp_duty_threshold_kobo,
    };
    let before_fee = retained - cut - before.collection_fee_share_kobo;
    let payout_fee = money::payout_fee_kobo(before_fee, &schedule);

    let (kind, amount, status): (String, i64, String) =
        sqlx::query_as("SELECT kind, amount_kobo, status::text FROM refunds WHERE order_id = $1")
            .bind(o.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), amount), ("partial", refund));
    assert_eq!(status, "processing", "refund sent after the commit");
    let (payout, fee): (i64, i64) =
        sqlx::query_as("SELECT amount_kobo, fee_kobo FROM payouts WHERE order_id = $1")
            .bind(o.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!((payout, fee), (before_fee - payout_fee, payout_fee));
    let cut_now: i64 = sqlx::query_scalar("SELECT platform_cut_kobo FROM orders WHERE id = $1")
        .bind(o.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(cut_now, cut);
    assert_ne!(cut_now, before.platform_cut_kobo);
    assert_eq!(order_status(&app, o.order_id).await, "completed");
    assert!(!held(&app, o.order_id).await);

    let d = dispute_row(&app, &id).await;
    assert_eq!(d.decision.as_deref(), Some("partial_refund"));
    assert_eq!(d.refund_kobo, Some(refund));
    let audits = decided_audits(&app).await;
    assert_eq!(audits.len(), 1);
    assert_eq!(
        audits[0].1,
        json!({ "dispute_id": id, "decision": "partial_refund", "refund_kobo": refund, "note": NOTE })
    );
}

#[sqlx::test]
async fn cannot_decide_before_seller_deadline(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("courier", "delivered").await;
    let id = open_id(&app, &o).await;
    for decision in ["release", "full_refund"] {
        let r = decide(
            &app,
            &id,
            &owner,
            json!({ "decision": decision, "note": NOTE }),
        )
        .await;
        assert_eq!(code(&r), (409, Some("awaiting_seller")), "{decision}");
    }
    let d = dispute_row(&app, &id).await;
    assert_eq!((d.status.as_str(), d.decision), ("awaiting_seller", None));
    assert_eq!(refunds(&app, o.order_id).await, 0);
    assert_eq!(payouts(&app, o.order_id).await, 0);
    assert!(held(&app, o.order_id).await);
    assert!(decided_audits(&app).await.is_empty());
}

#[sqlx::test]
async fn can_decide_after_deadline_without_response(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("courier", "delivered").await;
    let id = open_id(&app, &o).await;
    // Past the deadline, before the deadline job has moved it on.
    age_deadline(&app, &id).await;
    let r = decide(
        &app,
        &id,
        &owner,
        json!({ "decision": "release", "note": NOTE }),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.json);
    assert_eq!(r.json["status"], "decided");
    assert_eq!(r.json["seller_missed_deadline"], true);
    assert_eq!(payouts(&app, o.order_id).await, 1);
}

#[sqlx::test]
async fn support_cannot_decide(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let support = app.make_support().await;
    let o = app.shipped_order("courier", "delivered").await;
    let id = answered(&app, &o).await;
    for token in [&support, &o.buyer_token, &o.seller_token] {
        let r = decide(
            &app,
            &id,
            token,
            json!({ "decision": "full_refund", "note": NOTE }),
        )
        .await;
        assert_eq!(r.status, 403, "{}", r.json);
    }
    assert_eq!(dispute_row(&app, &id).await.status, "awaiting_decision");
    assert_eq!(refunds(&app, o.order_id).await, 0);
    // Unknown dispute: 404 for the owner.
    let owner = app.owner_token().await;
    let r = decide(
        &app,
        &Uuid::new_v4().to_string(),
        &owner,
        json!({ "decision": "release", "note": NOTE }),
    )
    .await;
    assert_eq!(r.status, 404);
}

#[sqlx::test]
async fn concurrent_decisions_act_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    app.fake.set_refund_supported(true);
    for _ in 0..3 {
        let o = app.shipped_order("courier", "delivered").await;
        let id = answered(&app, &o).await;
        let (a, b) = tokio::join!(
            decide(
                &app,
                &id,
                &owner,
                json!({ "decision": "release", "note": NOTE })
            ),
            decide(
                &app,
                &id,
                &owner,
                json!({ "decision": "full_refund", "note": NOTE })
            ),
        );
        let mut statuses = [a.status, b.status];
        statuses.sort();
        assert_eq!(statuses, [200, 409], "{} / {}", a.json, b.json);
        let loser = if a.status == 409 { &a } else { &b };
        assert_eq!(loser.json["error"]["code"], "dispute_closed");
        let (r, p) = (
            refunds(&app, o.order_id).await,
            payouts(&app, o.order_id).await,
        );
        assert_eq!(r + p, 1, "exactly one of refund ({r}) or payout ({p})");
        let d = dispute_row(&app, &id).await;
        let expected = if p == 1 { "release" } else { "full_refund" };
        assert_eq!(d.decision.as_deref(), Some(expected));
    }
    assert_eq!(decided_audits(&app).await.len(), 3);
}

#[sqlx::test]
async fn decision_stands_when_refund_provider_is_down(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("courier", "delivered").await;
    let id = answered(&app, &o).await;
    app.fake.set_refund_supported(true);
    app.fake
        .fail_refund_with(ProviderError::Unavailable(anyhow::anyhow!("down")));

    let r = decide(
        &app,
        &id,
        &owner,
        json!({ "decision": "full_refund", "note": NOTE }),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.json);
    assert_eq!(dispute_row(&app, &id).await.status, "decided");
    let (status, last_error): (String, Option<String>) =
        sqlx::query_as("SELECT status::text, last_error FROM refunds WHERE order_id = $1")
            .bind(o.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(status, "requested");
    assert_eq!(last_error.as_deref(), Some("unavailable"));
    assert_eq!(order_status(&app, o.order_id).await, "refund_pending");

    // The refund job sends it once the provider is back, same reference.
    app.fake.clear_refund_failure();
    sqlx::query(
        "UPDATE refunds SET updated_at = now() - interval '2 minutes', next_attempt_at = NULL",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    run_due_jobs(&app.state).await;
    assert_eq!(
        text(
            &app,
            "SELECT status::text FROM refunds WHERE order_id = $1",
            o.order_id
        )
        .await,
        "processing"
    );
    assert_eq!(order_status(&app, o.order_id).await, "refunded");
    let sent = app.fake.refunds();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].reference, sent[1].reference);
}

#[sqlx::test]
async fn plain_owner_routes_refuse_open_dispute(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("courier", "delivered").await;
    let id = open_id(&app, &o).await;
    for action in ["release", "refund", "unhold"] {
        let r = app
            .request(
                Method::POST,
                &format!("/admin/orders/{}/{action}", o.order_id),
                Some(json!({ "note": "Settling it directly" })),
                Some(&owner),
            )
            .await;
        assert_eq!(
            code(&r),
            (409, Some("dispute_open")),
            "{action}: {}",
            r.json
        );
    }
    assert!(held(&app, o.order_id).await);
    assert_eq!(order_status(&app, o.order_id).await, "delivered");
    assert_eq!(refunds(&app, o.order_id).await, 0);
    assert_eq!(payouts(&app, o.order_id).await, 0);
    assert_eq!(app.fake.refunds().len(), 0);
    assert_eq!(dispute_row(&app, &id).await.status, "awaiting_seller");

    // The same is true while the dispute waits for a decision or a return.
    age_deadline(&app, &id).await;
    run_due_jobs(&app.state).await;
    assert_eq!(dispute_row(&app, &id).await.status, "awaiting_decision");
    let r = app
        .request(
            Method::POST,
            &format!("/admin/orders/{}/release", o.order_id),
            Some(json!({ "note": "Settling it directly" })),
            Some(&owner),
        )
        .await;
    assert_eq!(code(&r), (409, Some("dispute_open")));
    assert_eq!(payouts(&app, o.order_id).await, 0);
}

#[sqlx::test]
async fn withdrawn_dispute_cannot_be_decided(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("courier", "delivered").await;
    let id = answered(&app, &o).await;
    let w = app
        .request(
            Method::POST,
            &format!("/disputes/{id}/withdraw"),
            Some(json!({ "confirm": true })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(w.status, 200, "{}", w.json);
    for decision in ["release", "full_refund"] {
        let r = decide(
            &app,
            &id,
            &owner,
            json!({ "decision": decision, "note": NOTE }),
        )
        .await;
        assert_eq!(code(&r), (409, Some("dispute_closed")), "{decision}");
        assert!(
            r.json["error"]["message"]
                .as_str()
                .unwrap()
                .contains("withdrawn")
        );
    }
    let r = decide(
        &app,
        &id,
        &owner,
        json!({ "decision": "partial_refund", "refund_kobo": 100, "note": NOTE }),
    )
    .await;
    assert_eq!(code(&r), (409, Some("dispute_closed")));
    assert_eq!(dispute_row(&app, &id).await.status, "withdrawn");
    assert_eq!(refunds(&app, o.order_id).await, 0);
    assert_eq!(payouts(&app, o.order_id).await, 0);
    assert!(decided_audits(&app).await.is_empty());
    // With no open dispute the plain owner routes work again.
    let r = app
        .request(
            Method::POST,
            &format!("/admin/orders/{}/hold", o.order_id),
            Some(json!({ "reason": "Owner check" })),
            Some(&owner),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.json);
    let r = app
        .request(
            Method::POST,
            &format!("/admin/orders/{}/unhold", o.order_id),
            Some(json!({ "note": "Checked" })),
            Some(&owner),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.json);
}
