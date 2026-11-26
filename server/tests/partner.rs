//! Part 9: the gateway (switch, budget, breaker, metering), the Owner's routes, the app
//! config flags and the signed webhook.

mod common;

use std::{sync::Arc, time::Duration};

use axum::http::Method;
use common::{PARTNER_WEBHOOK_SECRET, TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    escrow::jobs::run_due_jobs,
    partners::{
        PartnerError, PartnerGateway,
        client::NoPartner,
        webhook::{SIGNATURE_HEADER, TIMESTAMP_HEADER, sign},
    },
};
use sqlx::PgPool;

async fn owner(app: &TestApp) -> String {
    app.owner_token().await
}

async fn patch(app: &TestApp, token: &str, body: Value) -> TestResponse {
    app.request(Method::PATCH, "/admin/partner", Some(body), Some(token))
        .await
}

async fn turn_on(app: &TestApp, token: &str) {
    let res = patch(app, token, json!({"enabled": true, "note": "go"})).await;
    assert_eq!(res.status, 200, "{}", res.json);
}

async fn features(app: &TestApp) -> Value {
    let res = app.request(Method::GET, "/app/config", None, None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    res.json["features"].clone()
}

#[sqlx::test]
async fn the_marketplace_works_with_no_partner(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    // A server with no partner configured: every call says so and nothing else changes.
    let none = PartnerGateway::new(Arc::new(NoPartner), pool.clone(), None);
    assert!(!none.configured() && !none.available().await.unwrap());
    assert_eq!(
        none.get("price_history", "v1/x").await,
        Err(PartnerError::NotConfigured)
    );
    assert_eq!(
        none.post("price_history", "v1/x", "k", &json!({"updates": []}))
            .await,
        Err(PartnerError::NotConfigured)
    );
    assert_eq!(none.health().await, Err(PartnerError::NotConfigured));
    // The config route answers, with no features, and the rest of the app is untouched.
    assert_eq!(features(&app).await, json!({}));
    assert_eq!(
        app.request(Method::GET, "/health", None, None).await.status,
        200
    );
    // Even a configured partner is off until the Owner turns it on: no call is made.
    assert_eq!(
        app.state.partner.get("price_history", "v1/x").await,
        Err(PartnerError::Disabled)
    );
    assert_eq!(app.partner.calls(), 0);
}

#[sqlx::test]
async fn only_the_owner_changes_the_switches_and_every_change_is_audited_with_its_note(
    pool: PgPool,
) {
    let app = TestApp::new(pool).await;
    let o = owner(&app).await;
    let (_id, user) = app.sign_up_verified("08031112222").await;
    let support = app.make_support().await;
    for t in [&user, &support] {
        assert_eq!(
            patch(&app, t, json!({"enabled": true, "note": "x"}))
                .await
                .status,
            403
        );
        assert_eq!(
            app.request(Method::GET, "/admin/partner/status", None, Some(t))
                .await
                .status,
            403
        );
    }
    // A note is required, and nothing to change is refused.
    assert_eq!(patch(&app, &o, json!({"enabled": true})).await.status, 422);
    assert_eq!(
        patch(&app, &o, json!({"enabled": true, "note": "  "}))
            .await
            .status,
        422
    );
    assert_eq!(patch(&app, &o, json!({"note": "n"})).await.status, 422);
    assert_eq!(
        patch(
            &app,
            &o,
            json!({"features": {"Bad Name": true}, "note": "n"})
        )
        .await
        .status,
        422
    );
    assert_eq!(
        patch(&app, &o, json!({"monthly_call_budget": -1, "note": "n"}))
            .await
            .status,
        422
    );
    assert_eq!(
        patch(&app, &o, json!({"surprise": 1, "note": "n"}))
            .await
            .status,
        422
    );

    let res = patch(
        &app,
        &o,
        json!({"enabled": true, "features": {"price_history": true}, "monthly_call_budget": 100, "note": "pilot"}),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["enabled"], true);
    assert_eq!(res.json["monthly_call_budget"], 100);
    // A feature not named keeps its value; null removes the cap.
    let res = patch(
        &app,
        &o,
        json!({"features": {"comparison": false}, "monthly_call_budget": null, "note": "again"}),
    )
    .await;
    assert_eq!(
        res.json["features"],
        json!([{"name": "comparison", "on": false}, {"name": "price_history", "on": true}])
    );
    assert_eq!(res.json["monthly_call_budget"], Value::Null);
    let notes: Vec<Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = 'partner.settings_changed' ORDER BY id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(notes, [json!({"note": "pilot"}), json!({"note": "again"})]);
}

#[sqlx::test]
async fn flags_need_a_configured_enabled_partner_a_closed_breaker_and_budget_left(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = owner(&app).await;
    patch(&app, &o, json!({"features": {"price_history": true, "comparison": false}, "monthly_call_budget": 3, "note": "n"})).await;
    // Switched off: every flag reads false, the keys are still there.
    assert_eq!(
        features(&app).await,
        json!({"comparison": false, "price_history": false})
    );
    turn_on(&app, &o).await;
    assert_eq!(
        features(&app).await,
        json!({"comparison": false, "price_history": true})
    );

    // The breaker opens after five failures: flags read false at once.
    app.partner.set_all(Err(PartnerError::Unavailable));
    for _ in 0..3 {
        assert_eq!(
            app.state.partner.get("price_history", "v1/x").await,
            Err(PartnerError::Unavailable)
        );
    }
    // The budget (3) is spent by those three calls: flags false, and calls refused.
    assert_eq!(features(&app).await["price_history"], false);
    assert_eq!(
        app.state.partner.get("price_history", "v1/x").await,
        Err(PartnerError::BudgetSpent)
    );
    assert_eq!(app.partner.calls(), 3, "a spent budget stops the call");
}

#[sqlx::test]
async fn the_breaker_opens_after_five_failures_and_the_partner_is_not_called_again(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = owner(&app).await;
    patch(
        &app,
        &o,
        json!({"features": {"price_history": true}, "note": "n"}),
    )
    .await;
    turn_on(&app, &o).await;
    app.partner.set_all(Err(PartnerError::Unavailable));
    for _ in 0..5 {
        assert_eq!(
            app.state.partner.get("price_history", "v1/x").await,
            Err(PartnerError::Unavailable)
        );
    }
    assert_eq!(
        app.state.partner.get("price_history", "v1/x").await,
        Err(PartnerError::CircuitOpen)
    );
    assert_eq!(app.partner.calls(), 5);
    assert_eq!(features(&app).await["price_history"], false);
    let status = app
        .request(Method::GET, "/admin/partner/status", None, Some(&o))
        .await;
    assert_eq!(status.json["breaker"], "open");
    // The last error is the partner's own failure, not our refusal to call it.
    assert_eq!(status.json["last_error"], "unavailable");
    // The status route does not poke an open breaker.
    assert_eq!(status.json["connected"], false);
}

#[sqlx::test]
async fn a_partner_that_says_no_does_not_trip_the_breaker(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = owner(&app).await;
    turn_on(&app, &o).await;
    app.partner.set_all(Err(PartnerError::Rejected));
    for _ in 0..8 {
        assert_eq!(
            app.state.partner.get("price_history", "v1/x").await,
            Err(PartnerError::Rejected)
        );
    }
    assert_eq!(app.partner.calls(), 8);
    assert!(app.state.partner.available().await.unwrap());
}

#[sqlx::test]
async fn usage_is_metered_per_feature_and_day_and_errors_are_counted(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = owner(&app).await;
    turn_on(&app, &o).await;
    for _ in 0..3 {
        app.state
            .partner
            .get("price_history", "v1/x")
            .await
            .unwrap();
    }
    app.state.partner.get("comparison", "v1/y").await.unwrap();
    app.partner.set_next(Err(PartnerError::Unavailable));
    let _ = app.state.partner.get("comparison", "v1/y").await;
    let rows: Vec<(String, i32, i32)> =
        sqlx::query_as("SELECT feature, calls, errors FROM partner_usage ORDER BY feature")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(
        rows,
        [("comparison".into(), 2, 1), ("price_history".into(), 3, 0)]
    );
    let status = app
        .request(Method::GET, "/admin/partner/status", None, Some(&o))
        .await;
    assert_eq!(status.json["calls_this_month"], 5);
    assert_eq!(
        status.json["usage"][0],
        json!({"feature": "price_history", "calls": 3, "errors": 0})
    );
    // The connection check is not metered.
    assert_eq!(status.json["connected"], true);
    let n: i32 = sqlx::query_scalar("SELECT COALESCE(SUM(calls), 0)::int FROM partner_usage")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(n, 5);
}

#[sqlx::test]
async fn the_budget_stops_calls_and_raises_one_alert_a_month(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = owner(&app).await;
    patch(
        &app,
        &o,
        json!({"monthly_call_budget": 2, "enabled": true, "note": "cap"}),
    )
    .await;
    app.state
        .partner
        .get("price_history", "v1/x")
        .await
        .unwrap();
    app.state
        .partner
        .get("price_history", "v1/x")
        .await
        .unwrap();
    for _ in 0..4 {
        assert_eq!(
            app.state.partner.get("price_history", "v1/x").await,
            Err(PartnerError::BudgetSpent)
        );
    }
    assert_eq!(app.partner.calls(), 2);
    let alerts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'partner.budget_exhausted'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(alerts, 1, "many refusals, one alert");
    // The dashboard's alert list shows it, open.
    let list = app
        .request(Method::GET, "/admin/alerts?state=open", None, Some(&o))
        .await;
    assert!(
        list.json.to_string().contains("partner.budget_exhausted"),
        "{}",
        list.json
    );
    // Raising the cap lets calls through again.
    patch(&app, &o, json!({"monthly_call_budget": 10, "note": "more"})).await;
    assert!(app.state.partner.get("price_history", "v1/x").await.is_ok());
    // No cap at all.
    patch(
        &app,
        &o,
        json!({"monthly_call_budget": null, "note": "none"}),
    )
    .await;
    assert!(app.state.partner.get("price_history", "v1/x").await.is_ok());
}

#[sqlx::test]
async fn a_payload_with_a_personal_field_is_refused_and_costs_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = owner(&app).await;
    turn_on(&app, &o).await;
    let r = app
        .state
        .partner
        .post(
            "price_push",
            "v1/prices",
            "k1",
            &json!({"updates": [{"store_ref": "s", "buyer_name": "Ada"}]}),
        )
        .await;
    assert_eq!(r, Err(PartnerError::Blocked));
    assert_eq!(app.partner.calls(), 0);
    let used: i64 = sqlx::query_scalar("SELECT count(*) FROM partner_usage")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(used, 0);
    // A clean one goes out with its idempotency key.
    let ok = json!({"updates": [{"store_ref": "s", "barcode": "b", "price_kobo": 1000, "effective_at": "2026-11-26T10:00:00Z"}]});
    app.state
        .partner
        .post("price_push", "v1/prices", "k2", &ok)
        .await
        .unwrap();
    assert_eq!(app.partner.last_idempotency_key().as_deref(), Some("k2"));
    assert_eq!(app.partner.last_body(), Some(ok));
}

#[sqlx::test]
async fn a_slow_partner_never_holds_up_the_rest_of_the_app(pool: PgPool) {
    let app = Arc::new(TestApp::new(pool).await);
    let o = owner(&app).await;
    turn_on(&app, &o).await;
    app.partner.set_delay(Some(Duration::from_millis(600)));
    let a = app.clone();
    let slow = tokio::spawn(async move { a.state.partner.get("price_history", "v1/x").await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = std::time::Instant::now();
    let res = app.request(Method::GET, "/app/config", None, None).await;
    assert_eq!(res.status, 200);
    assert!(
        started.elapsed() < Duration::from_millis(300),
        "{:?}",
        started.elapsed()
    );
    assert!(slow.await.unwrap().is_ok());
}

// ---- the signed webhook ----

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

async fn hook(app: &TestApp, ts: i64, sig: &str, body: &[u8]) -> TestResponse {
    app.send_raw(
        Method::POST,
        "/webhooks/partner",
        &[
            (TIMESTAMP_HEADER, &ts.to_string()),
            (SIGNATURE_HEADER, sig),
            ("content-type", "application/json"),
        ],
        body.to_vec(),
    )
    .await
}

async fn signed(app: &TestApp, body: &Value) -> TestResponse {
    let raw = body.to_string().into_bytes();
    let ts = now();
    hook(app, ts, &sign(PARTNER_WEBHOOK_SECRET, ts, &raw), &raw).await
}

#[sqlx::test]
async fn a_signed_event_is_stored_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let event = json!({"event_id": "evt-1", "kind": "price.changed", "store_ref": "s1", "price_kobo": 5000});
    let first = signed(&app, &event).await;
    assert_eq!(first.status, 200, "{}", first.json);
    assert_eq!(first.json["status"], "received");
    let again = signed(&app, &event).await;
    assert_eq!(again.status, 200);
    assert_eq!(again.json["status"], "duplicate");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM partner_events")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    // A different event is its own row.
    signed(&app, &json!({"event_id": "evt-2", "kind": "price.stale"})).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM partner_events")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
}

#[sqlx::test]
async fn unsigned_or_badly_signed_webhooks_are_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let raw = json!({"event_id": "e", "kind": "price.changed"})
        .to_string()
        .into_bytes();
    let ts = now();
    let good = sign(PARTNER_WEBHOOK_SECRET, ts, &raw);
    // No headers, a wrong secret, a tampered body, a signature for another timestamp, junk.
    let none = app
        .send_raw(Method::POST, "/webhooks/partner", &[], raw.clone())
        .await;
    assert_eq!(none.status, 401);
    assert_eq!(none.json["error"]["code"], "invalid_signature");
    assert_eq!(
        hook(&app, ts, &sign("another-secret-entirely", ts, &raw), &raw)
            .await
            .status,
        401
    );
    let mut tampered = raw.clone();
    tampered.push(b' ');
    assert_eq!(hook(&app, ts, &good, &tampered).await.status, 401);
    assert_eq!(hook(&app, ts + 1, &good, &raw).await.status, 401);
    assert_eq!(hook(&app, ts, "zz", &raw).await.status, 401);
    assert_eq!(hook(&app, ts, "", &raw).await.status, 401);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM partner_events")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // And the right one passes.
    assert_eq!(hook(&app, ts, &good, &raw).await.status, 200);
}

#[sqlx::test]
async fn old_or_future_timestamps_are_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let raw = json!({"event_id": "e", "kind": "price.changed"})
        .to_string()
        .into_bytes();
    for offset in [-301, 301, -3600] {
        let ts = now() + offset;
        let r = hook(&app, ts, &sign(PARTNER_WEBHOOK_SECRET, ts, &raw), &raw).await;
        assert_eq!(r.status, 401, "offset {offset}");
    }
    // Inside the window, either way, is accepted.
    for (i, offset) in [-290, 290].into_iter().enumerate() {
        let raw = json!({"event_id": format!("w{i}"), "kind": "price.changed"})
            .to_string()
            .into_bytes();
        let ts = now() + offset;
        assert_eq!(
            hook(&app, ts, &sign(PARTNER_WEBHOOK_SECRET, ts, &raw), &raw)
                .await
                .status,
            200,
            "{offset}"
        );
    }
}

#[sqlx::test]
async fn an_oversize_body_and_a_malformed_one_are_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let big = format!(
        r#"{{"event_id":"big","kind":"price.changed","pad":"{}"}}"#,
        "x".repeat(70_000)
    )
    .into_bytes();
    let ts = now();
    let r = hook(&app, ts, &sign(PARTNER_WEBHOOK_SECRET, ts, &big), &big).await;
    assert_eq!(r.status, 413, "{}", r.json);
    assert_eq!(r.json["error"]["code"], "payload_too_large");
    // Signed, but not an event.
    for body in [
        b"not json".to_vec(),
        br#"{"kind":"price.changed"}"#.to_vec(),
        br#"{"event_id":"e","kind":"Bad Kind"}"#.to_vec(),
    ] {
        let r = hook(&app, ts, &sign(PARTNER_WEBHOOK_SECRET, ts, &body), &body).await;
        assert_eq!(r.status, 422, "{}", String::from_utf8_lossy(&body));
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM partner_events")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test]
async fn the_job_marks_events_processed_and_prunes_old_ones(pool: PgPool) {
    let app = TestApp::new(pool).await;
    signed(&app, &json!({"event_id": "a", "kind": "price.changed"})).await;
    let report = run_due_jobs(&app.state).await;
    assert_eq!(report.partner_events, 1);
    let open: i64 =
        sqlx::query_scalar("SELECT count(*) FROM partner_events WHERE processed_at IS NULL")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(open, 0);
    sqlx::query("UPDATE partner_events SET processed_at = now() - interval '31 days'")
        .execute(&app.pool)
        .await
        .unwrap();
    run_due_jobs(&app.state).await;
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM partner_events")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test]
async fn with_no_partner_the_webhook_is_not_found(pool: PgPool) {
    // A server with no secret has no such route to speak of.
    let mut app = TestApp::new(pool.clone()).await;
    app.state.partner = Arc::new(PartnerGateway::new(Arc::new(NoPartner), pool, None));
    // (The router was built with the old state; build one with the new one.)
    let none = server::build_app(app.state.clone());
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let res = none
        .oneshot(
            Request::post("/webhooks/partner")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 404);
}

#[sqlx::test]
async fn the_secrets_never_reach_a_response_or_an_audit_row(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = owner(&app).await;
    turn_on(&app, &o).await;
    let status = app
        .request(Method::GET, "/admin/partner/status", None, Some(&o))
        .await;
    let cfg = app.request(Method::GET, "/app/config", None, None).await;
    let audits: Vec<Value> = sqlx::query_scalar("SELECT detail FROM audit_log")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    for text in [
        status.json.to_string(),
        cfg.json.to_string(),
        json!(audits).to_string(),
    ] {
        assert!(!text.contains(PARTNER_WEBHOOK_SECRET), "{text}");
    }
}
