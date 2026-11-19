//! Owner re-book and retry-cancel for courier shipments (plan 6, task 7).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    couriers::CourierError,
    escrow::jobs::{JobReport, run_due_jobs},
    payments::PaymentStatus,
};
use sqlx::PgPool;
use uuid::Uuid;

const METHOD: &str = "mono_directpay";
const SELLER: &str = "08011111111";
const BUYER: &str = "08022222221";

struct Fixture {
    order_id: Uuid,
    seller: String,
}

/// A paid courier order, not yet marked ready.
async fn paid_order(app: &TestApp) -> Fixture {
    let listing = app
        .make_active_store_with_product(SELLER, "Mama Put", 10_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(
        app.set_cart(&buyer, listing.variant_id, 1).await.status,
        200
    );
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
    let res = app
        .checkout_with(&buyer, METHOD, address, &[(listing.store_id, option)])
        .await;
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
    let order_id = sqlx::query_scalar("SELECT id FROM orders WHERE checkout_id = $1")
        .bind(checkout_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    Fixture {
        order_id,
        seller: listing.seller_token,
    }
}

async fn ready(app: &TestApp, f: &Fixture) {
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/ready", f.order_id),
            None,
            Some(&f.seller),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
}

/// A paid courier order the seller has marked ready.
async fn ready_order(app: &TestApp) -> Fixture {
    let f = paid_order(app).await;
    ready(app, &f).await;
    f
}

async fn tick(app: &TestApp) -> JobReport {
    run_due_jobs(&app.state).await
}

#[derive(sqlx::FromRow)]
struct Row {
    status: String,
    last_error: Option<String>,
    provider_order_id: Option<String>,
}

async fn shipment(app: &TestApp, order_id: Uuid) -> Row {
    sqlx::query_as(
        "SELECT status::text AS status, last_error, provider_order_id \
         FROM shipments WHERE order_id = $1",
    )
    .bind(order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn status(app: &TestApp, order_id: Uuid) -> String {
    shipment(app, order_id).await.status
}

async fn make_due(app: &TestApp) {
    sqlx::query("UPDATE shipments SET next_attempt_at = now()")
        .execute(&app.pool)
        .await
        .unwrap();
}

async fn shipment_id(app: &TestApp, order_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM shipments WHERE order_id = $1")
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn rebook(app: &TestApp, token: &str, id: Uuid, body: Value) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/admin/shipments/{id}/rebook"),
        Some(body),
        Some(token),
    )
    .await
}

async fn retry_cancel(app: &TestApp, token: &str, id: Uuid) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/admin/shipments/{id}/retry-cancel"),
        Some(json!({"note": "wallet funded"})),
        Some(token),
    )
    .await
}

/// A ready order whose booking the courier refused (an empty wallet looks like this).
async fn failed_booking(app: &TestApp) -> (Fixture, Uuid) {
    let f = ready_order(app).await;
    app.courier.fail_book_with(CourierError::Rejected {
        code: "http_400".into(),
        message: "insufficient".into(),
    });
    tick(app).await;
    assert_eq!(status(app, f.order_id).await, "booking_failed");
    let id = shipment_id(app, f.order_id).await;
    (f, id)
}

#[sqlx::test]
async fn only_the_owner_can_rebook(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let support = app.make_support().await;
    let (f, id) = failed_booking(&app).await;
    assert_eq!(
        rebook(&app, &support, id, json!({"note": "x"}))
            .await
            .status,
        403
    );
    assert_eq!(
        rebook(&app, &f.seller, id, json!({"note": "x"}))
            .await
            .status,
        403
    );
    assert_eq!(status(&app, f.order_id).await, "booking_failed");
}

#[sqlx::test]
async fn a_failed_booking_is_booked_again_by_the_normal_job(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let (f, id) = failed_booking(&app).await;
    assert_eq!(app.courier.book_calls(), 1);

    // The wallet is funded; the owner re-books.
    app.courier.clear_book_failure();
    assert_eq!(
        rebook(&app, &owner, id, json!({})).await.status,
        422,
        "a note is required"
    );
    let res = rebook(&app, &owner, id, json!({"note": "Funded the wallet"})).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "booking_pending");
    assert_eq!(status(&app, f.order_id).await, "booking_pending");
    // Re-booking itself calls nobody: the job loop books it.
    assert_eq!(app.courier.book_calls(), 1);

    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booked", "{:?}", row.last_error);
    assert_eq!(app.courier.book_calls(), 2);
    assert!(row.provider_order_id.is_some());

    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_log WHERE action = 'admin.shipment_rebooked' AND subject_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);

    // A booked shipment cannot be re-booked.
    let again = rebook(&app, &owner, id, json!({"note": "again"})).await;
    assert_eq!(again.status, 409);
    assert_eq!(again.json["error"]["code"], "invalid_shipment_state");
}

#[sqlx::test]
async fn an_unknown_booking_needs_the_owner_to_confirm_nothing_was_booked(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let f = ready_order(&app).await;
    app.courier.fail_book_with(CourierError::Timeout);
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booking_unknown");
    let id = shipment_id(&app, f.order_id).await;
    app.courier.clear_book_failure();

    let blind = rebook(&app, &owner, id, json!({"note": "try again"})).await;
    assert_eq!(blind.status, 409, "{}", blind.json);
    assert_eq!(blind.json["error"]["code"], "confirm_not_booked");
    assert_eq!(status(&app, f.order_id).await, "booking_unknown");
    assert_eq!(app.courier.book_calls(), 1);

    let ok = rebook(
        &app,
        &owner,
        id,
        json!({"note": "No label for this order on the courier dashboard", "confirm_not_booked": true}),
    )
    .await;
    assert_eq!(ok.status, 200, "{}", ok.json);
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
    assert_eq!(app.courier.book_calls(), 2);
}

#[sqlx::test]
async fn a_shipment_with_a_label_is_never_rebooked(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let f = ready_order(&app).await;
    app.courier.fail_book_with(CourierError::Timeout);
    tick(&app).await;
    let id = shipment_id(&app, f.order_id).await;
    // The label turned up after all.
    sqlx::query("UPDATE shipments SET provider_order_id = 'SB-1' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let res = rebook(
        &app,
        &owner,
        id,
        json!({"note": "retry", "confirm_not_booked": true}),
    )
    .await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "label_exists");
}

#[sqlx::test]
async fn a_stale_quote_needs_a_new_quote_not_a_rebook(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let (f, id) = failed_booking(&app).await;
    for reason in ["quote_expired", "pickup_moved"] {
        sqlx::query("UPDATE shipments SET last_error = $2 WHERE id = $1")
            .bind(id)
            .bind(reason)
            .execute(&pool)
            .await
            .unwrap();
        let res = rebook(&app, &owner, id, json!({"note": "retry"})).await;
        assert_eq!(res.status, 409, "{reason}: {}", res.json);
        assert_eq!(res.json["error"]["code"], "requote_needed");
    }
    assert_eq!(status(&app, f.order_id).await, "booking_failed");
}

#[sqlx::test]
async fn a_refunded_or_unpaid_order_is_not_rebooked(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let (f, id) = failed_booking(&app).await;
    sqlx::query("UPDATE orders SET status = 'refunded' WHERE id = $1")
        .bind(f.order_id)
        .execute(&pool)
        .await
        .unwrap();
    let res = rebook(&app, &owner, id, json!({"note": "retry"})).await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "invalid_order_state");
}

#[sqlx::test]
async fn a_booking_in_flight_is_not_rebooked(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let f = ready_order(&app).await;
    tick(&app).await;
    let id = shipment_id(&app, f.order_id).await;
    sqlx::query(
        "UPDATE shipments SET status = 'booking', provider_order_id = NULL, \
                sending_until = now() + interval '2 minutes' WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let res = rebook(
        &app,
        &owner,
        id,
        json!({"note": "retry", "confirm_not_booked": true}),
    )
    .await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "booking_in_flight");
    assert_eq!(
        rebook(&app, &owner, Uuid::new_v4(), json!({"note": "x"}))
            .await
            .status,
        404
    );
}

#[sqlx::test]
async fn a_failed_cancel_is_retried(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let f = ready_order(&app).await;
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
    let id = shipment_id(&app, f.order_id).await;
    app.courier.fail_cancel_with(CourierError::Rejected {
        code: "http_400".into(),
        message: "no".into(),
    });
    sqlx::query(
        "UPDATE shipments SET status = 'cancel_pending', cancel_requested_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    make_due(&app).await;
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "cancel_failed");

    // Not a booking problem: rebook refuses it, retry-cancel takes it.
    assert_eq!(
        rebook(&app, &owner, id, json!({"note": "x"})).await.status,
        409
    );
    app.courier.clear_cancel_failure();
    let res = retry_cancel(&app, &owner, id).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "cancel_pending");
    make_due(&app).await;
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "cancelled");

    // Only a cancel_failed shipment can be retried.
    let again = retry_cancel(&app, &owner, id).await;
    assert_eq!(again.status, 409);
}
