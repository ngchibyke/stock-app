//! `/ready`: what an uptime monitor polls.

mod common;

use axum::http::Method;
use common::TestApp;
use serde_json::json;
use sqlx::PgPool;

async fn ready(app: &TestApp) -> common::TestResponse {
    app.request(Method::GET, "/ready", None, None).await
}

#[sqlx::test]
async fn a_healthy_server_says_ok_with_nothing_else(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = ready(&app).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json, json!({"status": "ok"}));
    // After a tick of the real jobs, still ok: nothing is stale or failing.
    server::escrow::jobs::run_due_jobs(&app.state).await;
    assert_eq!(ready(&app).await.status, 200);
    // It is public: no sign-in needed, and /health is still the plain liveness check.
    assert_eq!(
        app.request(Method::GET, "/health", None, None).await.status,
        200
    );
}

#[sqlx::test]
async fn a_job_that_stopped_running_is_named_as_stale(pool: PgPool) {
    let app = TestApp::new(pool).await;
    server::escrow::jobs::run_due_jobs(&app.state).await;
    sqlx::query(
        "UPDATE job_runs SET last_run_at = now() - interval '6 minutes' WHERE job = 'payouts'",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let res = ready(&app).await;
    assert_eq!(res.status, 503);
    assert_eq!(res.json["status"], "degraded");
    assert_eq!(res.json["database"], true);
    assert_eq!(res.json["stale_jobs"], json!(["payouts"]));
    assert_eq!(res.json["failing_jobs"], json!([]));
    // Within the allowance it is fine.
    sqlx::query(
        "UPDATE job_runs SET last_run_at = now() - interval '4 minutes' WHERE job = 'payouts'",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(ready(&app).await.status, 200);
}

#[sqlx::test]
async fn a_job_that_keeps_failing_is_named_but_one_odd_failure_is_not(pool: PgPool) {
    let app = TestApp::new(pool).await;
    server::escrow::jobs::run_due_jobs(&app.state).await;
    // One failure with a recent success: a blip, not an outage.
    sqlx::query(
        "UPDATE job_runs SET failed = 2, last_success_at = now() - interval '2 minutes' WHERE job = 'refunds'",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(ready(&app).await.status, 200);
    // No success for a long time while failing: named.
    sqlx::query(
        "UPDATE job_runs SET last_success_at = now() - interval '20 minutes' WHERE job = 'refunds'",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let res = ready(&app).await;
    assert_eq!(res.status, 503);
    assert_eq!(res.json["failing_jobs"], json!(["refunds"]));
    // Never succeeded and failing counts too.
    sqlx::query("UPDATE job_runs SET last_success_at = NULL WHERE job = 'refunds'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(ready(&app).await.json["failing_jobs"], json!(["refunds"]));
}

#[sqlx::test]
async fn an_unreachable_database_is_503_and_says_so_without_any_detail(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.pool.close().await;
    let res = ready(&app).await;
    assert_eq!(res.status, 503);
    assert_eq!(res.json["database"], false);
    // Only fixed fields: no error text, no connection details.
    let text = res.json.to_string();
    for leak in ["postgres", "market", "localhost", "pool", "error"] {
        assert!(!text.contains(leak), "{text}");
    }
    // And the liveness check still answers: the process is up.
    assert_eq!(
        app.request(Method::GET, "/health", None, None).await.status,
        200
    );
}

#[sqlx::test]
async fn a_fresh_server_with_no_job_history_is_ready(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM job_runs")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(ready(&app).await.status, 200);
}
