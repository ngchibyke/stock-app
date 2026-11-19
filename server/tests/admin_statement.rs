//! Daily statement reconciliation (plan 6, task 9).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use server::statement::{
    CheckOutcome, TOLERANCE_KOBO, Totals, check_day, job::run_for, ledger_totals,
};
use sqlx::PgPool;
use time::Date;
use tokio_util::sync::CancellationToken;

async fn today(app: &TestApp) -> Date {
    sqlx::query_scalar("SELECT (now() AT TIME ZONE 'Africa/Lagos')::date")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn get(app: &TestApp, path: &str, token: &str) -> TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

async fn post(app: &TestApp, path: &str, body: Value, token: &str) -> TestResponse {
    app.request(Method::POST, path, Some(body), Some(token))
        .await
}

const ZERO: Totals = Totals {
    collected_kobo: 0,
    paid_out_kobo: 0,
    refunded_kobo: 0,
    fees_kobo: 0,
};

async fn check(app: &TestApp, day: Date) -> CheckOutcome {
    check_day(&app.pool, app.statement.as_ref(), day, None)
        .await
        .unwrap()
}

async fn rows(app: &TestApp, status: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM statement_checks WHERE status = $1")
        .bind(status)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn the_ledger_side_adds_up_a_days_money(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let empty = ledger_totals(&app.pool, today(&app).await).await.unwrap();
    assert_eq!(empty, ZERO);

    // A paid order: money collected from the buyer, none paid out or refunded yet.
    app.shipped_order("self", "delivered").await;
    let t = ledger_totals(&app.pool, today(&app).await).await.unwrap();
    assert!(t.collected_kobo > 0, "{t:?}");
    assert_eq!((t.paid_out_kobo, t.refunded_kobo), (0, 0));
}

#[sqlx::test]
async fn equal_figures_match_and_nothing_is_raised(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    app.shipped_order("self", "delivered").await;
    let day = today(&app).await;
    let ledger = ledger_totals(&pool, day).await.unwrap();
    app.statement.set_day(day, ledger);

    assert_eq!(check(&app, day).await, CheckOutcome::Matched);
    assert_eq!(rows(&app, "matched").await, 1);
    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM audit_log WHERE action = 'statement.mismatch'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test]
async fn a_difference_beyond_the_tolerance_is_a_mismatch_and_an_alert(pool: PgPool) {
    let mut app = TestApp::new(pool.clone()).await;
    app.use_statement();
    let owner = app.owner_token().await;
    app.shipped_order("self", "delivered").await;
    let day = today(&app).await;
    let ledger = ledger_totals(&pool, day).await.unwrap();

    // Exactly the tolerance still matches; one kobo more does not.
    app.statement.set_day(
        day,
        Totals {
            fees_kobo: ledger.fees_kobo + TOLERANCE_KOBO,
            ..ledger
        },
    );
    assert_eq!(check(&app, day).await, CheckOutcome::Matched);
    app.statement.set_day(
        day,
        Totals {
            collected_kobo: ledger.collected_kobo - 5_000,
            ..ledger
        },
    );
    assert_eq!(check(&app, day).await, CheckOutcome::Mismatch);

    let (diff,): (Value,) =
        sqlx::query_as("SELECT diff FROM statement_checks WHERE status = 'mismatch'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(diff["collected_kobo"], -5_000);

    // The mismatch is an alert the owner can acknowledge.
    let alerts = get(&app, "/admin/alerts?state=open", &owner).await;
    let items = alerts.json["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{}", alerts.json);
    assert_eq!(items[0]["kind"], "statement.mismatch");
    let key = items[0]["alert_key"].as_str().unwrap();
    let ack = post(
        &app,
        "/admin/alerts/ack",
        json!({"alert_key": key, "note": "Mono confirmed a late reversal"}),
        &owner,
    )
    .await;
    assert_eq!(ack.status, 200, "{}", ack.json);
}

#[sqlx::test]
async fn an_unanswered_day_is_recorded_without_figures(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let day = today(&app).await;
    assert_eq!(check(&app, day).await, CheckOutcome::Unavailable);
    assert_eq!(rows(&app, "unavailable").await, 1);
    let (statement, diff): (Option<Value>, Option<Value>) =
        sqlx::query_as("SELECT statement, diff FROM statement_checks")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(statement.is_none() && diff.is_none());
}

#[sqlx::test]
async fn the_job_checks_the_last_three_days_once_each(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.use_statement();
    let t = today(&app).await;
    for back in 1..=3 {
        app.statement.set_day(t - time::Duration::days(back), ZERO);
    }
    let cancel = CancellationToken::new();
    let stats = run_for(&app.state, &cancel, 0).await.unwrap();
    assert_eq!((stats.done, stats.failed), (3, 0));
    assert_eq!(rows(&app, "matched").await, 3);
    // Today is not finished, and the matched days are not asked again.
    assert_eq!(app.statement.calls(), 3);
    let again = run_for(&app.state, &cancel, 0).await.unwrap();
    assert_eq!((again.done, again.failed), (0, 0));
    assert_eq!(app.statement.calls(), 3);
}

#[sqlx::test]
async fn the_job_waits_for_the_morning_and_retries_an_unanswered_day_after_an_hour(pool: PgPool) {
    let mut app = TestApp::new(pool.clone()).await;
    app.use_statement();
    let t = today(&app).await;
    let cancel = CancellationToken::new();

    // Before the earliest hour nothing runs (hour 24 is never reached).
    let early = run_for(&app.state, &cancel, 24).await.unwrap();
    assert_eq!((early.done, early.failed), (0, 0));
    assert_eq!(app.statement.calls(), 0);

    // The provider has only yesterday: two days fail, one matches.
    app.statement.set_day(t - time::Duration::days(1), ZERO);
    let first = run_for(&app.state, &cancel, 0).await.unwrap();
    assert_eq!((first.done, first.failed), (1, 2));
    let calls = app.statement.calls();
    // Not asked again straight away.
    run_for(&app.state, &cancel, 0).await.unwrap();
    assert_eq!(app.statement.calls(), calls);
    // An hour later the two days are tried again.
    sqlx::query("UPDATE statement_checks SET checked_at = now() - interval '2 hours' WHERE status = 'unavailable'")
        .execute(&pool)
        .await
        .unwrap();
    app.statement.set_day(t - time::Duration::days(2), ZERO);
    let retry = run_for(&app.state, &cancel, 0).await.unwrap();
    assert_eq!((retry.done, retry.failed), (1, 1));
}

#[sqlx::test]
async fn with_no_source_the_job_does_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let mut state = app.state.clone();
    state.statements = None;
    let stats = run_for(&state, &CancellationToken::new(), 0).await.unwrap();
    assert_eq!((stats.done, stats.failed), (0, 0));
    assert_eq!(
        rows(&app, "matched").await + rows(&app, "unavailable").await,
        0
    );
}

#[sqlx::test]
async fn the_owner_checks_a_finished_day_and_lists_the_results(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.use_statement();
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let t = today(&app).await;
    let yesterday = t - time::Duration::days(1);
    app.statement.set_day(yesterday, ZERO);

    assert_eq!(
        get(&app, "/admin/statement-checks", &support).await.status,
        403
    );
    assert_eq!(
        post(
            &app,
            "/admin/statement-checks",
            json!({"day": yesterday.to_string()}),
            &support
        )
        .await
        .status,
        403
    );
    let bad = post(
        &app,
        "/admin/statement-checks",
        json!({"day": "yesterday"}),
        &owner,
    )
    .await;
    assert_eq!(bad.status, 422);
    let open = post(
        &app,
        "/admin/statement-checks",
        json!({"day": t.to_string()}),
        &owner,
    )
    .await;
    assert_eq!(open.status, 422);
    assert_eq!(open.json["error"]["code"], "day_not_finished");

    let ok = post(
        &app,
        "/admin/statement-checks",
        json!({"day": yesterday.to_string()}),
        &owner,
    )
    .await;
    assert_eq!(ok.status, 200, "{}", ok.json);
    assert_eq!(ok.json["status"], "matched");
    assert_eq!(ok.json["day"], yesterday.to_string());

    // A day the provider cannot answer is recorded, not an error.
    let none = post(
        &app,
        "/admin/statement-checks",
        json!({"day": (t - time::Duration::days(5)).to_string()}),
        &owner,
    )
    .await;
    assert_eq!(none.status, 200);
    assert_eq!(none.json["status"], "unavailable");

    let list = get(&app, "/admin/statement-checks", &owner).await;
    assert_eq!(list.status, 200, "{}", list.json);
    assert_eq!(list.json["items"].as_array().unwrap().len(), 2);
}
