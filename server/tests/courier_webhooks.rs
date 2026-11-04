//! Courier status from signed webhooks and the polling fallback. A courier
//! `completed` starts the release window; it never moves money by itself.

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    couriers::{CourierUpdate, ShipmentState, fake::FAKE_SIGNATURE_HEADER},
    escrow::{
        jobs::{JobReport, run_due_jobs},
        refund::refund_order,
    },
    payments::PaymentStatus,
};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};
use tracing_test::traced_test;
use uuid::Uuid;

const METHOD: &str = "mono_directpay";
const SELLER: &str = "08011111111";
const BUYER: &str = "08022222221";
const LABEL: &str = "fake-1";
const HOOK: &str = "/webhooks/couriers/fake";
const COURIER_PHONE: &str = "08055501234";

struct Fixture {
    order_id: Uuid,
    seller: String,
}

/// Pays the checkout answered by `res`; returns the order id.
async fn pay(app: &TestApp, res: TestResponse) -> Uuid {
    assert_eq!(res.status, 201, "{}", res.json);
    let reference = res.json["reference"].as_str().unwrap().to_string();
    let total = res.json["total_kobo"].as_i64().unwrap();
    let checkout_id: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
    app.fake.set_payment(
        &reference,
        PaymentStatus::Successful {
            amount_kobo: total,
            fee_kobo: 10,
            currency: "NGN".into(),
            held: false,
            reference: reference.clone(),
        },
    );
    assert_eq!(
        settle_payment(&app.state, &reference).await.unwrap(),
        SettleOutcome::Paid
    );
    sqlx::query_scalar("SELECT id FROM orders WHERE checkout_id = $1")
        .bind(checkout_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// One paid order, delivered by the seller (`method` = "self") or by courier.
async fn paid_order(app: &TestApp, method: &str) -> Fixture {
    let listing = app
        .make_active_store_with_product(SELLER, "Mama Put", 10_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(
        app.set_cart(&buyer, listing.variant_id, 1).await.status,
        200
    );
    let res = if method == "self" {
        app.checkout(&buyer, METHOD).await
    } else {
        let address = app.buyer_address(&buyer).await;
        let opts = app.delivery_options(&buyer, address).await;
        assert_eq!(opts.status, 200, "{}", opts.json);
        let option: Uuid = opts.json["stores"][0]["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["method"] == "courier")
            .expect("a courier option")["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        app.checkout_with(&buyer, METHOD, address, &[(listing.store_id, option)])
            .await
    };
    let order_id = pay(app, res).await;
    Fixture {
        order_id,
        seller: listing.seller_token,
    }
}

async fn step(app: &TestApp, f: &Fixture, name: &str, body: Option<Value>) {
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/{name}", f.order_id),
            body,
            Some(&f.seller),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
}

/// A courier order the job loop has booked: shipment `booked`, label [`LABEL`].
async fn booked_order(app: &TestApp) -> Fixture {
    let f = paid_order(app, "courier").await;
    step(app, &f, "ready", None).await;
    run_due_jobs(&app.state).await;
    assert_eq!(shipment_status(app, f.order_id).await, "booked");
    f
}

async fn hook(app: &TestApp, body: Value) -> TestResponse {
    app.send_raw(
        Method::POST,
        HOOK,
        &[(FAKE_SIGNATURE_HEADER, "ok")],
        serde_json::to_vec(&body).unwrap(),
    )
    .await
}

/// Sends a status update for [`LABEL`] and insists on a 200.
async fn courier_says(app: &TestApp, state: &str) -> TestResponse {
    let res = hook(app, json!({ "provider_order_id": LABEL, "state": state })).await;
    assert_eq!(res.status, 200, "{}", res.json);
    res
}

async fn shipment_status(app: &TestApp, order_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM shipments WHERE order_id = $1")
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn order_status(app: &TestApp, order_id: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM orders WHERE id = $1")
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// `(source, status)` of the order's events, oldest first.
async fn events(app: &TestApp, order_id: Uuid) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT source, status FROM delivery_events WHERE order_id = $1 \
         ORDER BY received_at, id",
    )
    .bind(order_id)
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

/// The order's events from the courier: their statuses, oldest first.
async fn courier_events(app: &TestApp, order_id: Uuid) -> Vec<String> {
    events(app, order_id)
        .await
        .into_iter()
        .filter(|(source, _)| source == "courier")
        .map(|(_, status)| status)
        .collect()
}

async fn audits(app: &TestApp, action: &str) -> Vec<Value> {
    sqlx::query_scalar("SELECT detail FROM audit_log WHERE action = $1 ORDER BY at, id")
        .bind(action)
        .fetch_all(&app.pool)
        .await
        .unwrap()
}

async fn payouts(app: &TestApp, order_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM payouts WHERE order_id = $1")
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn reconciliation(app: &TestApp) -> Value {
    let owner = app.owner_token().await;
    let res = app
        .request(Method::GET, "/admin/reconciliation", None, Some(&owner))
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    res.json
}

/// The shipment's attention reason on the owner's list, if it is listed.
async fn listed(app: &TestApp, order_id: Uuid) -> Option<Value> {
    reconciliation(app).await["shipments_attention"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["order_id"] == order_id.to_string())
        .map(|s| s["attention"].clone())
}

/// A held order refunded by the owner: the only refund of a shipped order.
async fn owner_refund(app: &TestApp, order_id: Uuid) {
    let owner = app.owner_token().await;
    let held = app
        .request(
            Method::POST,
            &format!("/admin/orders/{order_id}/hold"),
            Some(json!({ "reason": "buyer complaint" })),
            Some(&owner),
        )
        .await;
    assert_eq!(held.status, 200, "{}", held.json);
    let refunded = app
        .request(
            Method::POST,
            &format!("/admin/orders/{order_id}/refund"),
            Some(json!({ "note": "parcel never came" })),
            Some(&owner),
        )
        .await;
    assert_eq!(refunded.status, 200, "{}", refunded.json);
}

/// Makes the shipment look quiet for 7 hours.
async fn age(app: &TestApp, order_id: Uuid) {
    sqlx::query(
        "UPDATE shipments SET updated_at = now() - interval '7 hours', \
                last_event_at = NULL, last_polled_at = NULL WHERE order_id = $1",
    )
    .bind(order_id)
    .execute(&app.pool)
    .await
    .unwrap();
}

fn update(state: ShipmentState) -> CourierUpdate {
    CourierUpdate {
        provider_order_id: LABEL.into(),
        state,
        occurred_at: None,
        courier_phone: None,
        tracking_url: None,
    }
}

async fn tick(app: &TestApp) -> JobReport {
    run_due_jobs(&app.state).await
}

// ------------------------------------------------------------ forward only

#[sqlx::test]
async fn courier_statuses_only_move_forward(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    assert_eq!(
        courier_says(&app, "completed").await.json["status"],
        "received"
    );
    assert_eq!(
        courier_says(&app, "picked_up").await.json["status"],
        "ignored"
    );
    assert_eq!(order_status(&app, f.order_id).await, "delivered");
    assert_eq!(shipment_status(&app, f.order_id).await, "completed");
    assert_eq!(
        courier_events(&app, f.order_id).await,
        ["completed", "picked_up"]
    );
}

#[sqlx::test]
async fn a_same_state_update_with_a_new_time_is_recorded_and_ignored(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "picked_up").await;
    let res = hook(
        &app,
        json!({ "provider_order_id": LABEL, "state": "picked_up",
                "occurred_at": "2026-11-01T10:00:00Z" }),
    )
    .await;
    assert_eq!(res.json["status"], "ignored");
    assert_eq!(
        courier_events(&app, f.order_id).await,
        ["picked_up", "picked_up"]
    );
    assert_eq!(audits(&app, "order.picked_up").await.len(), 1);
}

#[sqlx::test]
async fn confirmed_moves_the_shipment_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "pending").await;
    assert_eq!(shipment_status(&app, f.order_id).await, "booked");
    courier_says(&app, "confirmed").await;
    assert_eq!(shipment_status(&app, f.order_id).await, "confirmed");
    assert_eq!(order_status(&app, f.order_id).await, "paid");
}

#[sqlx::test]
async fn completed_after_buyer_confirmed_leaves_the_order_completed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "picked_up").await;
    sqlx::query("UPDATE orders SET status = 'completed' WHERE id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    courier_says(&app, "completed").await;
    assert_eq!(order_status(&app, f.order_id).await, "completed");
    assert_eq!(shipment_status(&app, f.order_id).await, "completed");
}

// ------------------------------------------------------ the release window

#[sqlx::test]
async fn completed_starts_release_window_not_release(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "picked_up").await;
    // A back-dated status must not shorten the window.
    let before = OffsetDateTime::now_utc();
    let long_ago = before - Duration::days(5);
    let res = hook(
        &app,
        json!({ "provider_order_id": LABEL, "state": "completed",
                "occurred_at": long_ago.format(&time::format_description::well_known::Rfc3339).unwrap() }),
    )
    .await;
    assert_eq!(res.status, 200);
    let after = OffsetDateTime::now_utc();
    let (delivered_at, release_at): (OffsetDateTime, OffsetDateTime) =
        sqlx::query_as("SELECT delivered_at, release_at FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(delivered_at >= before - Duration::seconds(1) && delivered_at <= after);
    assert_eq!(release_at - delivered_at, Duration::hours(72));
    assert_eq!(order_status(&app, f.order_id).await, "delivered");
    assert_eq!(payouts(&app, f.order_id).await, 0);
    // The event keeps what the provider said.
    let occurred: Option<OffsetDateTime> = sqlx::query_scalar(
        "SELECT occurred_at FROM delivery_events WHERE order_id = $1 AND status = 'completed'",
    )
    .bind(f.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(occurred.is_some_and(|o| o < before));
}

#[sqlx::test]
async fn completed_on_a_paid_order_walks_to_delivered_with_no_payout(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    assert_eq!(order_status(&app, f.order_id).await, "paid");
    courier_says(&app, "completed").await;
    assert_eq!(order_status(&app, f.order_id).await, "delivered");
    let steps: (bool, bool, bool) = sqlx::query_as(
        "SELECT picked_up_at IS NOT NULL, out_for_delivery_at IS NOT NULL, \
                delivered_at IS NOT NULL FROM orders WHERE id = $1",
    )
    .bind(f.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(steps, (true, true, true));
    assert_eq!(courier_events(&app, f.order_id).await, ["completed"]);
    // Nothing is due for 72 hours, whatever the job loop does.
    let report = tick(&app).await;
    assert_eq!(report.released, 0);
    assert_eq!(payouts(&app, f.order_id).await, 0);
    assert_eq!(order_status(&app, f.order_id).await, "delivered");
    let ledger: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ledger_entries WHERE order_id = $1 AND account = 'seller_payable'",
    )
    .bind(f.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(ledger, 0, "no seller credit before the window ends");
}

#[sqlx::test]
async fn in_transit_on_a_paid_order_walks_to_out_for_delivery(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "in_transit").await;
    assert_eq!(order_status(&app, f.order_id).await, "out_for_delivery");
    assert_eq!(shipment_status(&app, f.order_id).await, "in_transit");
    let code: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT handover_code_enc FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(code.is_none(), "courier orders have no handover code");
    courier_says(&app, "completed").await;
    assert_eq!(order_status(&app, f.order_id).await, "delivered");
}

#[sqlx::test]
async fn a_self_order_is_never_delivered_by_a_courier_update(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    step(&app, &f, "picked-up", None).await;
    step(&app, &f, "out-for-delivery", None).await;
    sqlx::query(
        "INSERT INTO shipments (order_id, provider, status, provider_order_id, quoted_kobo) \
         VALUES ($1, 'fake', 'booked', $2, 0)",
    )
    .bind(f.order_id)
    .bind(LABEL)
    .execute(&app.pool)
    .await
    .unwrap();
    courier_says(&app, "completed").await;
    assert_eq!(order_status(&app, f.order_id).await, "out_for_delivery");
    let delivered_at: Option<OffsetDateTime> =
        sqlx::query_scalar("SELECT delivered_at FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(delivered_at.is_none());
    assert_eq!(
        listed(&app, f.order_id).await,
        Some(json!("not_courier_order"))
    );
}

// ------------------------------------------------------------- idempotence

#[sqlx::test]
async fn duplicate_webhook_is_deduped(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    let body = json!({ "provider_order_id": LABEL, "state": "picked_up",
                       "occurred_at": "2026-11-01T10:00:00Z" });
    assert_eq!(hook(&app, body.clone()).await.json["status"], "received");
    let again = hook(&app, body).await;
    assert_eq!(again.status, 200);
    assert_eq!(again.json["status"], "ignored");
    assert_eq!(courier_events(&app, f.order_id).await, ["picked_up"]);
    assert_eq!(audits(&app, "order.picked_up").await.len(), 1);
}

#[sqlx::test]
async fn the_same_instant_in_another_offset_is_a_duplicate(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    for at in ["2026-11-01T10:00:00Z", "2026-11-01T11:00:00+01:00"] {
        hook(
            &app,
            json!({ "provider_order_id": LABEL, "state": "picked_up", "occurred_at": at }),
        )
        .await;
    }
    assert_eq!(courier_events(&app, f.order_id).await, ["picked_up"]);
}

#[sqlx::test]
async fn concurrent_duplicates_land_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    let body = json!({ "provider_order_id": LABEL, "state": "completed" });
    let (a, b) = tokio::join!(hook(&app, body.clone()), hook(&app, body));
    assert_eq!((a.status, b.status), (200, 200));
    assert_eq!(courier_events(&app, f.order_id).await, ["completed"]);
    assert_eq!(audits(&app, "order.delivered").await.len(), 1);
}

// ------------------------------------------------------------- the route

#[sqlx::test]
async fn bad_signature_is_401_and_nothing_written(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    let body =
        serde_json::to_vec(&json!({ "provider_order_id": LABEL, "state": "completed" })).unwrap();
    for headers in [vec![], vec![(FAKE_SIGNATURE_HEADER, "wrong")]] {
        let res = app
            .send_raw(Method::POST, HOOK, &headers, body.clone())
            .await;
        assert_eq!(res.status, 401, "{}", res.json);
        assert_eq!(res.json["error"]["code"], "invalid_signature");
    }
    assert!(
        events(&app, f.order_id)
            .await
            .iter()
            .all(|e| e.0 != "courier")
    );
    assert_eq!(order_status(&app, f.order_id).await, "paid");
    assert_eq!(shipment_status(&app, f.order_id).await, "booked");
}

#[sqlx::test]
async fn another_providers_name_is_404(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = app
        .send_raw(
            Method::POST,
            "/webhooks/couriers/shipbubble",
            &[(FAKE_SIGNATURE_HEADER, "ok")],
            b"{}".to_vec(),
        )
        .await;
    assert_eq!(res.status, 404);
}

#[sqlx::test]
async fn the_body_is_capped_at_64_kib(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = app
        .send_raw(
            Method::POST,
            HOOK,
            &[(FAKE_SIGNATURE_HEADER, "ok")],
            vec![b' '; 64 * 1024 + 1],
        )
        .await;
    assert_eq!(res.status, 413);
    assert_eq!(res.json["error"]["code"], "payload_too_large");
}

#[sqlx::test]
#[traced_test]
async fn unknown_shipment_is_200_and_logged(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = hook(
        &app,
        json!({ "provider_order_id": "nobody", "state": "completed" }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(logs_contain("unknown_shipment"));
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM delivery_events")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test]
async fn a_signed_body_we_cannot_read_is_200_ignored(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = app
        .send_raw(
            Method::POST,
            HOOK,
            &[(FAKE_SIGNATURE_HEADER, "ok")],
            b"not json".to_vec(),
        )
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json["status"], "ignored");
    let unknown_status = hook(
        &app,
        json!({ "provider_order_id": LABEL, "state": "teleported" }),
    )
    .await;
    assert_eq!(unknown_status.json["status"], "ignored");
}

#[sqlx::test]
async fn a_busy_order_is_503_and_the_retry_lands(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    let mut holder = app.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM orders WHERE id = $1 FOR UPDATE")
        .bind(f.order_id)
        .execute(&mut *holder)
        .await
        .unwrap();
    let res = hook(
        &app,
        json!({ "provider_order_id": LABEL, "state": "picked_up" }),
    )
    .await;
    assert_eq!(res.status, 503, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "order_busy");
    holder.rollback().await.unwrap();
    assert!(courier_events(&app, f.order_id).await.is_empty());
    courier_says(&app, "picked_up").await;
    assert_eq!(order_status(&app, f.order_id).await, "picked_up");
}

#[sqlx::test]
async fn wallet_alert_is_audited_with_the_event_name_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = hook(
        &app,
        json!({ "event": "wallet.threshold.alert", "amount": 123456 }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let rows = audits(&app, "courier.wallet_alert").await;
    assert_eq!(rows, [json!({ "event": "wallet.threshold.alert" })]);
    let json = reconciliation(&app).await;
    assert_eq!(json["wallet_alerts"][0]["event"], "wallet.threshold.alert");
    assert!(!json.to_string().contains("123456"));
}

// ----------------------------------------------------------- personal data

#[sqlx::test]
#[traced_test]
async fn webhook_logs_no_body_or_phone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    booked_order(&app).await;
    let res = hook(
        &app,
        json!({ "provider_order_id": LABEL, "state": "picked_up",
                "courier_phone": COURIER_PHONE,
                "tracking_url": "https://track.fake.test/secret-path" }),
    )
    .await;
    assert_eq!(res.json["status"], "received");
    // Unknown shipment and ignored update go down the logging paths too.
    hook(
        &app,
        json!({ "provider_order_id": "nobody", "state": "completed",
                "courier_phone": COURIER_PHONE }),
    )
    .await;
    assert_eq!(
        courier_says(&app, "pending").await.json["status"],
        "ignored"
    );
    assert!(logs_contain("unknown_shipment"));
    for secret in [COURIER_PHONE, "secret-path", "nobody"] {
        assert!(!logs_contain(secret), "{secret} was logged");
    }
}

#[sqlx::test]
async fn courier_phone_is_stored_encrypted(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    hook(
        &app,
        json!({ "provider_order_id": LABEL, "state": "picked_up",
                "courier_phone": COURIER_PHONE }),
    )
    .await;
    let enc: Vec<u8> =
        sqlx::query_scalar("SELECT courier_phone_enc FROM shipments WHERE order_id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(
        !enc.windows(COURIER_PHONE.len())
            .any(|w| w == COURIER_PHONE.as_bytes()),
        "stored in the clear"
    );
    assert_eq!(app.state.cipher.decrypt(&enc).unwrap(), COURIER_PHONE);
    // A later update with no phone keeps it.
    courier_says(&app, "in_transit").await;
    let kept: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT courier_phone_enc FROM shipments WHERE order_id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(kept.is_some());
}

#[sqlx::test]
async fn only_an_https_tracking_url_is_stored(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    let url = || async {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT tracking_url FROM shipments WHERE order_id = $1",
        )
        .bind(f.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
    };
    let booked = url().await;
    hook(
        &app,
        json!({ "provider_order_id": LABEL, "state": "confirmed",
                "tracking_url": "http://track.fake.test/plain" }),
    )
    .await;
    assert_eq!(url().await, booked);
    hook(
        &app,
        json!({ "provider_order_id": LABEL, "state": "picked_up",
                "tracking_url": "javascript:alert(1)" }),
    )
    .await;
    assert_eq!(url().await, booked);
    hook(
        &app,
        json!({ "provider_order_id": LABEL, "state": "in_transit",
                "tracking_url": "https://track.fake.test/new" }),
    )
    .await;
    assert_eq!(url().await.as_deref(), Some("https://track.fake.test/new"));
}

// ------------------------------------------------------------ cancellation

#[sqlx::test]
async fn cancel_after_pickup_is_ignored(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "picked_up").await;
    assert_eq!(
        courier_says(&app, "cancelled").await.json["status"],
        "ignored"
    );
    assert_eq!(shipment_status(&app, f.order_id).await, "picked_up");
    assert_eq!(order_status(&app, f.order_id).await, "picked_up");
    assert_eq!(
        courier_events(&app, f.order_id).await,
        ["picked_up", "cancelled"]
    );
}

#[sqlx::test]
async fn cancel_before_pickup_keeps_the_order_paid_and_ready(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    assert_eq!(
        courier_says(&app, "cancelled").await.json["status"],
        "received"
    );
    assert_eq!(shipment_status(&app, f.order_id).await, "cancelled");
    assert_eq!(order_status(&app, f.order_id).await, "paid");
    let ready: Option<OffsetDateTime> =
        sqlx::query_scalar("SELECT ready_at FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(ready.is_some());
    assert_eq!(
        listed(&app, f.order_id).await,
        Some(json!("courier_cancelled"))
    );
}

#[sqlx::test]
async fn completed_after_a_courier_cancel_still_delivers(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "cancelled").await;
    // Paperwork cannot undo it, and stale paperwork cannot revive it.
    courier_says(&app, "confirmed").await;
    assert_eq!(shipment_status(&app, f.order_id).await, "cancelled");
    courier_says(&app, "completed").await;
    assert_eq!(shipment_status(&app, f.order_id).await, "completed");
    assert_eq!(order_status(&app, f.order_id).await, "delivered");
}

#[sqlx::test]
async fn cancelled_ends_a_cancel_pending_shipment(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    assert_eq!(shipment_status(&app, f.order_id).await, "cancel_pending");
    courier_says(&app, "cancelled").await;
    assert_eq!(shipment_status(&app, f.order_id).await, "cancelled");
    assert_eq!(listed(&app, f.order_id).await, None);
    assert!(audits(&app, "shipment.after_refund").await.is_empty());
}

#[sqlx::test]
async fn cancelled_ends_a_cancel_failed_shipment(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    sqlx::query("UPDATE shipments SET status = 'cancel_failed' WHERE order_id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    courier_says(&app, "cancelled").await;
    assert_eq!(shipment_status(&app, f.order_id).await, "cancelled");
    assert_eq!(listed(&app, f.order_id).await, None);
}

#[sqlx::test]
async fn pickup_during_a_cancel_is_a_lost_race(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    let refunded = order_status(&app, f.order_id).await;
    for (i, state) in ["picked_up", "in_transit", "completed"].iter().enumerate() {
        courier_says(&app, state).await;
        assert_eq!(shipment_status(&app, f.order_id).await, "cancel_pending");
        assert_eq!(order_status(&app, f.order_id).await, refunded);
        assert_eq!(audits(&app, "shipment.cancel_lost").await.len(), i + 1);
    }
    assert_eq!(
        courier_events(&app, f.order_id).await,
        ["picked_up", "in_transit", "completed"]
    );
    assert_eq!(listed(&app, f.order_id).await, Some(json!("cancel_lost")));
    assert_eq!(payouts(&app, f.order_id).await, 0);
}

#[sqlx::test]
async fn pickup_after_a_failed_cancel_is_a_lost_race(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    sqlx::query("UPDATE shipments SET status = 'cancel_failed' WHERE order_id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    courier_says(&app, "picked_up").await;
    assert_eq!(shipment_status(&app, f.order_id).await, "cancel_failed");
    assert_eq!(listed(&app, f.order_id).await, Some(json!("cancel_lost")));
}

// ---------------------------------------------------------------- refunds

#[sqlx::test]
async fn refund_of_an_en_route_shipment_is_audited_and_listed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "in_transit").await;
    owner_refund(&app, f.order_id).await;
    assert_eq!(shipment_status(&app, f.order_id).await, "in_transit");
    let rows = audits(&app, "shipment.live_on_refund").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "in_transit");
    assert_eq!(
        listed(&app, f.order_id).await,
        Some(json!("en_route_on_refund"))
    );
}

#[sqlx::test]
async fn refund_of_a_picked_up_shipment_is_listed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "picked_up").await;
    owner_refund(&app, f.order_id).await;
    assert_eq!(
        listed(&app, f.order_id).await,
        Some(json!("en_route_on_refund"))
    );
}

#[sqlx::test]
async fn an_update_after_refund_never_moves_the_order(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "picked_up").await;
    owner_refund(&app, f.order_id).await;
    let refunded = order_status(&app, f.order_id).await;
    assert!(
        ["refund_pending", "refunded"].contains(&refunded.as_str()),
        "{refunded}"
    );
    courier_says(&app, "completed").await;
    assert_eq!(shipment_status(&app, f.order_id).await, "completed");
    assert_eq!(order_status(&app, f.order_id).await, refunded);
    let rows = audits(&app, "shipment.after_refund").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "completed");
    assert_eq!(rows[0]["order_status"], refunded.as_str());
    assert_eq!(listed(&app, f.order_id).await, Some(json!("after_refund")));
    assert_eq!(payouts(&app, f.order_id).await, 0);
}

#[sqlx::test]
async fn completed_after_a_cancelled_refund_records_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    courier_says(&app, "cancelled").await;
    let refunded = order_status(&app, f.order_id).await;
    courier_says(&app, "completed").await;
    assert_eq!(shipment_status(&app, f.order_id).await, "completed");
    assert_eq!(order_status(&app, f.order_id).await, refunded);
    assert_eq!(listed(&app, f.order_id).await, Some(json!("after_refund")));
}

// ---------------------------------------------------------------- polling

#[sqlx::test]
async fn poll_moves_stuck_shipment(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    age(&app, f.order_id).await;
    app.courier.push_update(update(ShipmentState::PickedUp));
    let report = tick(&app).await;
    assert_eq!(report.shipments_polled, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(app.courier.status_calls(), 1);
    assert_eq!(shipment_status(&app, f.order_id).await, "picked_up");
    assert_eq!(order_status(&app, f.order_id).await, "picked_up");
    assert_eq!(courier_events(&app, f.order_id).await, ["picked_up"]);
    // Fresh again: the next tick leaves it alone.
    tick(&app).await;
    assert_eq!(app.courier.status_calls(), 1);
}

#[sqlx::test]
async fn poll_completed_uses_our_time_and_pays_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    age(&app, f.order_id).await;
    app.courier.push_update(CourierUpdate {
        occurred_at: Some(OffsetDateTime::now_utc() - Duration::days(9)),
        ..update(ShipmentState::Completed)
    });
    tick(&app).await;
    assert_eq!(order_status(&app, f.order_id).await, "delivered");
    let window_open: bool = sqlx::query_scalar(
        "SELECT release_at > now() + interval '71 hours' FROM orders WHERE id = $1",
    )
    .bind(f.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(window_open);
    assert_eq!(payouts(&app, f.order_id).await, 0);
}

#[sqlx::test]
async fn poll_leaves_a_recent_shipment_alone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    booked_order(&app).await;
    tick(&app).await;
    assert_eq!(app.courier.status_calls(), 0);
}

#[sqlx::test]
async fn poll_counts_a_recent_event_as_recent(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "pending").await; // ignored, but the courier is talking
    sqlx::query("UPDATE shipments SET updated_at = now() - interval '7 hours' WHERE order_id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    tick(&app).await;
    assert_eq!(app.courier.status_calls(), 0);
}

#[sqlx::test]
async fn an_unchanged_poll_waits_another_six_hours(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    age(&app, f.order_id).await;
    tick(&app).await;
    assert_eq!(app.courier.status_calls(), 1);
    assert_eq!(shipment_status(&app, f.order_id).await, "booked");
    tick(&app).await;
    assert_eq!(app.courier.status_calls(), 1);
}

#[sqlx::test]
async fn poll_is_capped_at_20_a_tick_and_skips_finished_shipments(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    age(&app, f.order_id).await;
    // Cheap rows: one store's copies of one checkout's order.
    sqlx::query("ALTER TABLE orders DROP CONSTRAINT orders_one_per_store")
        .execute(&app.pool)
        .await
        .unwrap();
    for n in 0..24 {
        // Copies of the one order, each with a booked shipment gone quiet.
        let copy = Uuid::new_v4();
        sqlx::raw_sql(&format!(
            "CREATE TEMP TABLE o_copy AS SELECT * FROM orders WHERE id = '{id}'; \
             UPDATE o_copy SET id = '{copy}', delivery_option_id = NULL; \
             INSERT INTO orders SELECT * FROM o_copy; \
             DROP TABLE o_copy; \
             INSERT INTO shipments (order_id, provider, status, provider_order_id, quoted_kobo, \
                                    updated_at) \
             VALUES ('{copy}', 'fake', '{status}', 'copy-{n}', 0, now() - interval '7 hours');",
            id = f.order_id,
            status = if n < 22 { "booked" } else { "completed" },
        ))
        .execute(&app.pool)
        .await
        .unwrap();
    }
    tick(&app).await;
    assert_eq!(app.courier.status_calls(), 20);
    tick(&app).await;
    assert_eq!(app.courier.status_calls(), 23, "the rest, then nothing");
}

// ------------------------------------------------------------ the event log

#[sqlx::test]
async fn delivery_events_are_append_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = booked_order(&app).await;
    courier_says(&app, "picked_up").await;
    for sql in [
        "UPDATE delivery_events SET status = 'x'",
        "DELETE FROM delivery_events",
        "TRUNCATE delivery_events",
    ] {
        let err = sqlx::query(sql).execute(&app.pool).await.unwrap_err();
        assert!(err.to_string().contains("append-only"), "{sql}: {err}");
    }
    // Replication mode switches ordinary triggers off; these stay on.
    let mut tx = app.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query("DELETE FROM delivery_events")
            .execute(&mut *tx)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    assert!(!events(&app, f.order_id).await.is_empty());
}

#[sqlx::test]
async fn seller_steps_append_events_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    step(&app, &f, "picked-up", None).await;
    step(&app, &f, "picked-up", None).await;
    step(&app, &f, "out-for-delivery", None).await;
    step(&app, &f, "out-for-delivery", None).await;
    let enc: Vec<u8> = sqlx::query_scalar("SELECT handover_code_enc FROM orders WHERE id = $1")
        .bind(f.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let code = app.state.cipher.decrypt(&enc).unwrap();
    step(
        &app,
        &f,
        "delivered",
        Some(json!({ "handover_code": code })),
    )
    .await;
    let seller = |s: &str| ("seller".to_string(), s.to_string());
    assert_eq!(
        events(&app, f.order_id).await,
        [
            seller("picked_up"),
            seller("out_for_delivery"),
            seller("delivered")
        ]
    );
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT dedupe_key FROM delivery_events ORDER BY received_at, id")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(keys[0], format!("seller:{}:picked_up", f.order_id));
}

#[sqlx::test]
async fn ready_appends_one_seller_event(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "courier").await;
    step(&app, &f, "ready", None).await;
    step(&app, &f, "ready", None).await;
    assert_eq!(
        events(&app, f.order_id).await,
        [("seller".to_string(), "ready".to_string())]
    );
}

#[sqlx::test]
async fn a_failed_step_appends_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/out-for-delivery", f.order_id),
            None,
            Some(&f.seller),
        )
        .await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert!(events(&app, f.order_id).await.is_empty());
}
