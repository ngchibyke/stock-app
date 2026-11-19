//! Alert acknowledgement (plan 6, task 5).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn get(app: &TestApp, path: &str, token: &str) -> TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

async fn ack(app: &TestApp, token: &str, key: &str, note: &str) -> TestResponse {
    app.request(
        Method::POST,
        "/admin/alerts/ack",
        Some(json!({ "alert_key": key, "note": note })),
        Some(token),
    )
    .await
}

fn keys(v: &Value) -> Vec<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["alert_key"].as_str().unwrap().to_string())
        .collect()
}

/// An audit alert row `days` old; returns its audit id.
async fn audit_alert(pool: &PgPool, action: &str, subject: Uuid, days: i32, detail: Value) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO audit_log (at, action, subject_type, subject_id, detail) \
         VALUES (now() - make_interval(days => $4), $1, 'payout', $2, $3) RETURNING id",
    )
    .bind(action)
    .bind(subject)
    .bind(detail)
    .bind(days)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn mismatch() -> Value {
    json!({ "expected_kobo": 9_000, "reported_kobo": 8_000 })
}

#[sqlx::test]
async fn an_old_alert_stays_open_until_it_is_acknowledged(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let id = audit_alert(
        &pool,
        "payout.amount_mismatch",
        Uuid::new_v4(),
        40,
        mismatch(),
    )
    .await;
    let key = format!("audit:{id}");

    let open = get(&app, "/admin/alerts?state=open", &owner).await;
    assert_eq!(open.status, 200, "{}", open.json);
    assert_eq!(keys(&open.json), vec![key.clone()]);
    assert_eq!(open.json["items"][0]["kind"], "payout.amount_mismatch");

    // The reconciliation view lists it too, with the same key.
    let rec = get(&app, "/admin/reconciliation", &owner).await;
    assert_eq!(rec.json["payout_amount_mismatches"][0]["alert_key"], key);

    let done = ack(&app, &owner, &key, "Checked with Mono, amounts match").await;
    assert_eq!(done.status, 200, "{}", done.json);

    assert!(keys(&get(&app, "/admin/alerts?state=open", &owner).await.json).is_empty());
    let acked = get(&app, "/admin/alerts?state=acked", &owner).await;
    assert_eq!(keys(&acked.json), vec![key.clone()]);
    assert_eq!(
        acked.json["items"][0]["note"],
        "Checked with Mono, amounts match"
    );
    let rec = get(&app, "/admin/reconciliation", &owner).await;
    assert!(
        rec.json["payout_amount_mismatches"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test]
async fn acknowledging_needs_a_note_the_owner_and_a_real_open_alert(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let id = audit_alert(
        &pool,
        "courier.wallet_alert",
        Uuid::new_v4(),
        1,
        json!({"event": "x"}),
    )
    .await;
    let key = format!("audit:{id}");

    assert_eq!(ack(&app, &owner, &key, "   ").await.status, 422);
    assert_eq!(ack(&app, &support, &key, "ok").await.status, 403);
    assert_eq!(get(&app, "/admin/alerts", &support).await.status, 403);
    assert_eq!(ack(&app, &owner, "garbage", "ok").await.status, 422);
    assert_eq!(ack(&app, &owner, "audit:999999", "ok").await.status, 404);
    // Not an alert action.
    let other: i64 = sqlx::query_scalar(
        "INSERT INTO audit_log (action, subject_type) VALUES ('admin.viewed', 'admin_view') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        ack(&app, &owner, &format!("audit:{other}"), "ok")
            .await
            .status,
        404
    );

    assert_eq!(ack(&app, &owner, &key, "ok").await.status, 200);
    let again = ack(&app, &owner, &key, "ok").await;
    assert_eq!(again.status, 409);
    assert_eq!(again.json["error"]["code"], "already_acked");

    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM audit_log WHERE action = 'admin.alert_acked'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn acks_cannot_be_changed_or_deleted(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let id = audit_alert(&pool, "courier.wallet_alert", Uuid::new_v4(), 1, json!({})).await;
    ack(&app, &owner, &format!("audit:{id}"), "ok").await;
    assert!(
        sqlx::query("DELETE FROM alert_acks")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE alert_acks SET note = 'x'")
            .execute(&pool)
            .await
            .is_err()
    );
}

#[sqlx::test]
async fn repeated_handover_lockouts_raise_an_alert_that_returns_on_the_next(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let order = Uuid::new_v4();
    let lock = |p: PgPool| async move {
        sqlx::query(
            "INSERT INTO audit_log (action, subject_type, subject_id, detail) \
             VALUES ('order.handover_attempt', 'order', $1, '{\"ok\":false,\"locked\":true}')",
        )
        .bind(order)
        .execute(&p)
        .await
        .unwrap();
    };
    // A wrong code that did not lock does not count.
    sqlx::query(
        "INSERT INTO audit_log (action, subject_type, subject_id, detail) \
         VALUES ('order.handover_attempt', 'order', $1, '{\"ok\":false,\"locked\":false}')",
    )
    .bind(order)
    .execute(&pool)
    .await
    .unwrap();
    lock(pool.clone()).await;
    lock(pool.clone()).await;
    assert!(keys(&get(&app, "/admin/alerts", &owner).await.json).is_empty());

    lock(pool.clone()).await;
    let open = get(&app, "/admin/alerts", &owner).await;
    assert_eq!(
        open.json["items"].as_array().unwrap().len(),
        1,
        "{}",
        open.json
    );
    assert_eq!(
        open.json["items"][0]["kind"],
        "order.repeated_handover_lockout"
    );
    assert_eq!(open.json["items"][0]["subject_id"], order.to_string());
    let key = keys(&open.json).remove(0);
    assert_eq!(
        ack(&app, &owner, &key, "Spoke to the seller").await.status,
        200
    );
    assert!(keys(&get(&app, "/admin/alerts", &owner).await.json).is_empty());

    // The next lockout raises it again, under a new key.
    lock(pool.clone()).await;
    let again = keys(&get(&app, "/admin/alerts", &owner).await.json);
    assert_eq!(again.len(), 1);
    assert_ne!(again[0], key);
}

#[sqlx::test]
async fn a_paid_order_without_a_delivery_method_can_be_acknowledged(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("self", "picked_up").await;
    sqlx::query("UPDATE orders SET status = 'paid', delivery_method = NULL WHERE id = $1")
        .bind(o.order_id)
        .execute(&pool)
        .await
        .unwrap();

    let rec = get(&app, "/admin/reconciliation", &owner).await;
    let row = &rec.json["orders_attention"][0];
    assert_eq!(row["order_id"], o.order_id.to_string());
    let key = row["alert_key"].as_str().unwrap().to_string();
    assert!(keys(&get(&app, "/admin/alerts", &owner).await.json).contains(&key));

    assert_eq!(
        ack(&app, &owner, &key, "Contacted the buyer").await.status,
        200
    );
    let rec = get(&app, "/admin/reconciliation", &owner).await;
    assert!(rec.json["orders_attention"].as_array().unwrap().is_empty());
}

#[sqlx::test]
async fn an_unknown_courier_label_can_be_acknowledged(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    sqlx::query(
        "INSERT INTO audit_log (action, subject_type, detail) \
         VALUES ('courier.unknown_shipment', 'shipment', \
                 '{\"provider_order_id\": \"SB-123\", \"state\": \"created\"}')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let rec = get(&app, "/admin/reconciliation", &owner).await;
    let row = &rec.json["shipments_attention"][0];
    assert_eq!(row["attention"], "unknown_label");
    let key = row["alert_key"].as_str().unwrap().to_string();
    assert_eq!(
        ack(&app, &owner, &key, "Label belongs to a test booking")
            .await
            .status,
        200
    );
    let rec = get(&app, "/admin/reconciliation", &owner).await;
    assert!(
        rec.json["shipments_attention"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
