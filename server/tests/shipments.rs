//! Courier booking from the job loop: booking debits a prepaid wallet and the
//! provider's POST is not idempotent, so a wrong retry books and pays twice.

mod common;

use std::time::Duration;

use axum::http::Method;
use common::TestApp;
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    couriers::{CourierError, Rate, fake::default_rate},
    delivery::shipments::book_one,
    escrow::{
        jobs::{JobReport, run_due_jobs},
        refund::refund_order,
    },
    payments::PaymentStatus,
};
use sqlx::PgPool;
use tracing_test::traced_test;
use uuid::Uuid;

const METHOD: &str = "mono_directpay";
const SELLER: &str = "08011111111";
const BUYER: &str = "08022222221";
/// What the fake courier quotes.
const QUOTE: i64 = 150_000;
const COURIER_PHONE: &str = "08055501234";

struct Fixture {
    order_id: Uuid,
    store_id: Uuid,
    checkout_id: Uuid,
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
        store_id: listing.store_id,
        checkout_id,
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
    attempts: i32,
    last_error: Option<String>,
    provider_order_id: Option<String>,
    charged_kobo: Option<i64>,
}

async fn shipment(app: &TestApp, order_id: Uuid) -> Row {
    sqlx::query_as(
        "SELECT status::text AS status, attempts, last_error, provider_order_id, charged_kobo \
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

async fn shipment_count(app: &TestApp, order_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM shipments WHERE order_id = $1")
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn make_due(app: &TestApp) {
    sqlx::query("UPDATE shipments SET next_attempt_at = now()")
        .execute(&app.pool)
        .await
        .unwrap();
}

/// Seconds until the shipment is next due.
async fn wait_secs(app: &TestApp, order_id: Uuid) -> f64 {
    sqlx::query_scalar(
        "SELECT extract(epoch FROM next_attempt_at - now())::float8 \
         FROM shipments WHERE order_id = $1",
    )
    .bind(order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn audits(app: &TestApp, action: &str) -> Vec<Value> {
    sqlx::query_scalar("SELECT detail FROM audit_log WHERE action = $1 ORDER BY at, id")
        .bind(action)
        .fetch_all(&app.pool)
        .await
        .unwrap()
}

/// The order's `courier_charge` ledger entries, in posting order.
async fn charge_entries(app: &TestApp, order_id: Uuid) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT account::text, amount_kobo FROM ledger_entries \
         WHERE order_id = $1 AND kind = 'courier_charge' ORDER BY id",
    )
    .bind(order_id)
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

/// Puts the shipment in `booking` with a live lease, as a worker mid-call would.
async fn hold_lease(app: &TestApp, order_id: Uuid) {
    sqlx::query(
        "UPDATE shipments SET status = 'booking', \
                sending_until = now() + interval '2 minutes' WHERE order_id = $1",
    )
    .bind(order_id)
    .execute(&app.pool)
    .await
    .unwrap();
}

async fn attention(app: &TestApp, token: &str) -> Value {
    let res = app
        .request(Method::GET, "/admin/reconciliation", None, Some(token))
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    res.json
}

/// The seller's `PATCH /stores/{id}`.
async fn patch_store(app: &TestApp, f: &Fixture, body: Value) {
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{}", f.store_id),
            Some(body),
            Some(&f.seller),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
}

async fn pickup_code(app: &TestApp, f: &Fixture) -> Option<i64> {
    sqlx::query_scalar("SELECT pickup_courier_address_code FROM stores WHERE id = $1")
        .bind(f.store_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn shipment_id(app: &TestApp, order_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM shipments WHERE order_id = $1")
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// Waits for the fake to have received `n` cancel calls.
async fn cancel_started(app: &TestApp, n: usize) {
    for _ in 0..500 {
        if app.courier.cancel_calls() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("cancel was not called");
}

/// Waits for the fake to have received `n` book calls.
async fn book_started(app: &TestApp, n: usize) {
    for _ in 0..500 {
        if app.courier.book_calls() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("book was not called");
}

#[sqlx::test]
async fn ready_order_gets_booked_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booking_pending");
    let quoted: i64 = sqlx::query_scalar("SELECT quoted_kobo FROM shipments WHERE order_id = $1")
        .bind(f.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(quoted, QUOTE);

    // A repeated ready step does not add a row.
    ready(&app, &f).await;
    assert_eq!(shipment_count(&app, f.order_id).await, 1);

    let report = tick(&app).await;
    assert_eq!(report.shipments_booked, 1);
    assert_eq!(report.failed, 0);
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booked");
    assert_eq!(row.attempts, 1);
    assert_eq!(row.provider_order_id.as_deref(), Some("fake-1"));
    assert_eq!(row.charged_kobo, Some(QUOTE));
    assert_eq!(row.last_error, None);
    let (url, name, provider): (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT tracking_url, courier_name, provider FROM shipments WHERE order_id = $1",
    )
    .bind(f.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(url.as_deref(), Some("https://track.fake.test/fake-1"));
    assert_eq!(name.as_deref(), Some("Fake Courier"));
    assert_eq!(provider.as_deref(), Some("fake"));

    assert_eq!(tick(&app).await.shipments_booked, 0);
    assert_eq!(app.courier.book_calls(), 1);
    assert_eq!(audits(&app, "shipment.booked").await.len(), 1);
}

#[sqlx::test]
async fn job_creates_a_shipment_for_a_ready_order_without_one(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app).await;
    sqlx::query("UPDATE orders SET ready_at = now() WHERE id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(shipment_count(&app, f.order_id).await, 0);

    assert_eq!(tick(&app).await.shipments_booked, 1);
    assert_eq!(status(&app, f.order_id).await, "booked");
    assert_eq!(shipment_count(&app, f.order_id).await, 1);
}

#[sqlx::test]
async fn unready_order_gets_no_shipment(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app).await;
    tick(&app).await;
    assert_eq!(shipment_count(&app, f.order_id).await, 0);
    assert_eq!(app.courier.book_calls(), 0);
}

#[sqlx::test]
async fn booking_timeout_never_rebooks(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.fail_book_with(CourierError::Timeout);
    for _ in 0..3 {
        tick(&app).await;
        make_due(&app).await;
    }
    assert_eq!(app.courier.book_calls(), 1);
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booking_unknown");
    assert_eq!(row.provider_order_id, None);
    assert_eq!(audits(&app, "shipment.booking_unknown").await.len(), 1);
}

#[sqlx::test]
async fn inconclusive_booking_is_never_retried(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.fail_book_with(CourierError::Inconclusive);
    for _ in 0..3 {
        tick(&app).await;
        make_due(&app).await;
    }
    assert_eq!(app.courier.book_calls(), 1);
    assert_eq!(status(&app, f.order_id).await, "booking_unknown");
}

#[sqlx::test]
async fn booking_unavailable_backs_off_then_fails_after_8(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.fail_book_with(CourierError::Unavailable);

    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booking_pending");
    assert_eq!(row.attempts, 1);
    // Not due yet: the next tick leaves it alone.
    tick(&app).await;
    assert_eq!(app.courier.book_calls(), 1);

    let first = wait_secs(&app, f.order_id).await;
    assert!((290.0..=361.0).contains(&first), "first wait {first}");
    make_due(&app).await;
    tick(&app).await;
    let second = wait_secs(&app, f.order_id).await;
    assert!((590.0..=721.0).contains(&second), "second wait {second}");

    for attempt in 3..=7 {
        make_due(&app).await;
        tick(&app).await;
        let row = shipment(&app, f.order_id).await;
        assert_eq!(row.attempts, attempt);
        assert_eq!(row.status, "booking_pending");
        // Doubling, capped at 6 hours.
        assert!(wait_secs(&app, f.order_id).await <= 21_600.0);
    }
    make_due(&app).await;
    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booking_failed");
    assert_eq!(row.attempts, 8);
    assert_eq!(row.last_error.as_deref(), Some("unavailable"));
    assert_eq!(app.courier.book_calls(), 8);

    make_due(&app).await;
    tick(&app).await;
    assert_eq!(app.courier.book_calls(), 8);
}

#[sqlx::test]
async fn rejected_booking_fails_and_stores_only_the_code(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.fail_book_with(CourierError::Rejected {
        code: "http_400".into(),
        message: "provider words".into(),
    });
    tick(&app).await;
    make_due(&app).await;
    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booking_failed");
    assert_eq!(row.last_error.as_deref(), Some("http_400"));
    assert_eq!(app.courier.book_calls(), 1);
    // A 400 may be an empty wallet (the two cannot be told apart).
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    assert_eq!(json["wallet_alerts"].as_array().unwrap().len(), 1, "{json}");
    assert_eq!(json["wallet_alerts"][0]["event"], "possible_low_balance");
    assert!(!json.to_string().contains("provider words"));
}

#[sqlx::test]
async fn booked_price_above_tolerance_is_flagged(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    // The default 10% tolerance allows 165,000.
    app.courier.set_rates(vec![Rate {
        charged_kobo: 165_001,
        ..default_rate()
    }]);
    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booked");
    assert_eq!(row.charged_kobo, Some(165_001));
    let flags = audits(&app, "shipment.price_above_quote").await;
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0]["quoted"], QUOTE);
    assert_eq!(flags[0]["charged"], 165_001);

    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    let items = json["shipments_attention"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{json}");
    assert_eq!(items[0]["price_above_quote"], true);
    assert_eq!(items[0]["charged_kobo"], 165_001);
}

#[sqlx::test]
async fn booked_price_at_tolerance_is_not_flagged(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.set_rates(vec![Rate {
        charged_kobo: 165_000,
        ..default_rate()
    }]);
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
    assert!(audits(&app, "shipment.price_above_quote").await.is_empty());
}

#[sqlx::test]
async fn quote_older_than_7_days_fails_booking(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    sqlx::query(
        "UPDATE delivery_options SET created_at = now() - interval '7 days' \
         WHERE id = (SELECT delivery_option_id FROM orders WHERE id = $1)",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booking_failed");
    assert_eq!(row.last_error.as_deref(), Some("quote_expired"));
    assert_eq!(app.courier.book_calls(), 0);
}

#[sqlx::test]
async fn quote_under_7_days_still_books(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    sqlx::query(
        "UPDATE delivery_options SET created_at = now() - interval '6 days 23 hours' \
         WHERE id = (SELECT delivery_option_id FROM orders WHERE id = $1)",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
}

#[sqlx::test]
async fn pickup_moved_never_calls_book(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    patch_store(&app, &f, json!({ "pickup_area_council": "bwari" })).await;
    tick(&app).await;
    make_due(&app).await;
    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booking_failed");
    assert_eq!(row.last_error.as_deref(), Some("pickup_moved"));
    assert_eq!(app.courier.book_calls(), 0);

    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    assert_eq!(json["shipments_attention"][0]["last_error"], "pickup_moved");
}

#[sqlx::test]
async fn cleared_pickup_code_counts_as_moved(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    patch_store(&app, &f, json!({ "pickup_address": "9 New Road, Garki" })).await;
    assert_eq!(pickup_code(&app, &f).await, None);
    // A later quote refills the code; the order was still quoted from the old place.
    sqlx::query("UPDATE stores SET pickup_courier_address_code = 777 WHERE id = $1")
        .bind(f.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    tick(&app).await;
    assert_eq!(
        shipment(&app, f.order_id).await.last_error.as_deref(),
        Some("pickup_moved")
    );
    assert_eq!(app.courier.book_calls(), 0);
}

#[sqlx::test]
async fn moved_postcode_counts_as_moved(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    patch_store(&app, &f, json!({ "pickup_postcode": "FC01AB12CDE" })).await;
    tick(&app).await;
    assert_eq!(
        shipment(&app, f.order_id).await.last_error.as_deref(),
        Some("pickup_moved")
    );
    assert_eq!(app.courier.book_calls(), 0);
}

/// The address is unchanged, so the courier still goes to the right place; the
/// old number stays on the label, which the audit row records.
#[sqlx::test]
async fn phone_change_still_books(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    patch_store(&app, &f, json!({ "business_phone": "08031119999" })).await;
    assert_eq!(pickup_code(&app, &f).await, None);
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
    assert_eq!(app.courier.book_calls(), 1);
    let started = audits(&app, "shipment.booking_started").await;
    assert_eq!(started[0]["sender_stale"], true);
}

#[sqlx::test]
async fn resaving_the_same_address_still_books(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    patch_store(
        &app,
        &f,
        json!({
            "pickup_address": "  12 Unique Street, Wuse 2 ",
            "pickup_area_council": "amac"
        }),
    )
    .await;
    // Still cleared, as before: only the booking decision changes.
    assert_eq!(pickup_code(&app, &f).await, None);
    let moved: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT pickup_moved_at FROM stores WHERE id = $1")
            .bind(f.store_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(moved, None);
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
}

#[sqlx::test]
async fn move_before_the_quote_still_books(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    sqlx::query("UPDATE stores SET pickup_moved_at = now() - interval '1 day' WHERE id = $1")
        .bind(f.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
    let started = audits(&app, "shipment.booking_started").await;
    assert_eq!(started[0]["sender_stale"], false);
}

/// A process that died mid-call leaves `booking` behind. The POST may have
/// landed, so the row is never reclaimed.
#[sqlx::test]
async fn stale_booking_lease_goes_unknown_and_never_rebooks(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    sqlx::query(
        "UPDATE shipments SET status = 'booking', attempts = 1, \
                sending_until = now() - interval '1 second' WHERE order_id = $1",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    for _ in 0..3 {
        tick(&app).await;
        make_due(&app).await;
    }
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booking_unknown");
    assert_eq!(row.last_error.as_deref(), Some("lease_expired"));
    assert_eq!(app.courier.book_calls(), 0);
    assert_eq!(audits(&app, "shipment.booking_unknown").await.len(), 1);
}

#[sqlx::test]
async fn live_booking_lease_is_left_alone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    hold_lease(&app, f.order_id).await;
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booking");
    assert_eq!(app.courier.book_calls(), 0);
}

/// An answer arriving after the lease was lost must not overwrite the row, and
/// must leave the owner a way to find the label it created.
#[sqlx::test]
async fn late_booking_answer_does_not_overwrite(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.delay_next_book(Duration::from_millis(400));
    let running = {
        let state = app.state.clone();
        tokio::spawn(async move { run_due_jobs(&state).await })
    };
    book_started(&app, 1).await;
    // What the reaper does once the lease has run out.
    sqlx::query(
        "UPDATE shipments SET status = 'booking_unknown', sending_until = NULL, \
                last_error = 'lease_expired' WHERE order_id = $1",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    running.await.unwrap();

    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booking_unknown");
    assert_eq!(row.last_error.as_deref(), Some("lease_expired"));
    assert_eq!(row.provider_order_id, None);
    let late = audits(&app, "shipment.late_outcome").await;
    assert_eq!(late.len(), 1);
    assert_eq!(late[0]["provider_order_id"], "fake-1");

    // The paid label reaches the owner, not only the audit log.
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    let items = json["shipments_attention"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{json}");
    assert_eq!(items[0]["late_provider_order_id"], "fake-1");
}

/// A lost-lease answer stays listed whatever the row does next.
#[sqlx::test]
async fn late_label_stays_listed_after_the_row_moves_on(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    tick(&app).await;
    let id = shipment_id(&app, f.order_id).await;
    sqlx::query(
        "INSERT INTO audit_log (action, subject_type, subject_id, detail) \
         VALUES ('shipment.late_outcome', 'shipment', $1, '{\"provider_order_id\": \"fake-9\"}')",
    )
    .bind(id)
    .execute(&app.pool)
    .await
    .unwrap();
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    assert_eq!(
        json["shipments_attention"][0]["late_provider_order_id"], "fake-9",
        "{json}"
    );
}

/// The label exists but the row could not be written: the log is all that is left.
#[traced_test]
#[sqlx::test]
async fn unrecorded_booking_logs_the_label(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    sqlx::raw_sql(
        "CREATE FUNCTION refuse_booked() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'write refused'; END $$; \
         CREATE TRIGGER refuse_booked BEFORE UPDATE ON shipments FOR EACH ROW \
             WHEN (NEW.status = 'booked') EXECUTE FUNCTION refuse_booked();",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let report = tick(&app).await;
    assert_eq!(report.failed, 1);
    assert_eq!(status(&app, f.order_id).await, "booking");
    assert!(logs_contain("fake-1"), "label id not logged");
}

/// Two instances claiming the same ids: the order lock lets one through.
#[sqlx::test]
async fn two_ticks_at_once_book_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.delay_next_book(Duration::from_millis(200));
    let (a, b) = tokio::join!(run_due_jobs(&app.state), run_due_jobs(&app.state));
    assert_eq!(a.failed + b.failed, 0);
    assert_eq!(a.shipments_booked + b.shipments_booked, 1);
    assert_eq!(app.courier.book_calls(), 1);
    assert_eq!(status(&app, f.order_id).await, "booked");
}

/// `book_one` re-checks the row: a claim is no promise it is still due.
#[sqlx::test]
async fn book_one_refuses_a_row_no_longer_due(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    let id = shipment_id(&app, f.order_id).await;

    // Held between the claim and the call.
    sqlx::query("UPDATE orders SET held_at = now(), hold_reason = 'x' WHERE id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let booked = book_one(&app.state, app.courier.as_ref(), id, 1000).await;
    assert!(!booked.unwrap());
    assert_eq!(status(&app, f.order_id).await, "booking_pending");
    assert_eq!(app.courier.book_calls(), 0);

    // Leased by another worker.
    sqlx::query("UPDATE orders SET held_at = NULL, hold_reason = NULL WHERE id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    hold_lease(&app, f.order_id).await;
    let booked = book_one(&app.state, app.courier.as_ref(), id, 1000).await;
    assert!(!booked.unwrap());
    assert_eq!(status(&app, f.order_id).await, "booking");
    assert_eq!(app.courier.book_calls(), 0);
}

#[sqlx::test]
async fn booked_without_a_charge_is_on_the_attention_list(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    // No rate: the fake reports no charged amount, as an unreadable fee would.
    app.courier.set_rates(vec![]);
    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booked");
    assert_eq!(row.charged_kobo, None);
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    assert_eq!(json["shipments_attention"][0]["status"], "booked", "{json}");
}

/// The attention list is the only record of a label with no known charge: it
/// must stay there once a refund has moved the shipment on.
#[sqlx::test]
async fn cancelled_without_a_charge_stays_on_the_attention_list(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.set_rates(vec![]);
    tick(&app).await;
    assert_eq!(shipment(&app, f.order_id).await.status, "booked");
    assert!(charge_entries(&app, f.order_id).await.is_empty());

    refund_order(&app.state, f.order_id, true).await.unwrap();
    tick(&app).await;
    assert_eq!(shipment(&app, f.order_id).await.status, "cancelled");
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    assert_eq!(
        json["shipments_attention"][0]["status"], "cancelled",
        "{json}"
    );
    assert!(charge_entries(&app, f.order_id).await.is_empty());
}

#[sqlx::test]
async fn a_booking_books_its_charge_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    assert!(charge_entries(&app, f.order_id).await.is_empty());
    tick(&app).await;
    let booked = vec![
        ("courier_costs".to_string(), QUOTE),
        ("platform_revenue".to_string(), -QUOTE),
    ];
    assert_eq!(charge_entries(&app, f.order_id).await, booked);
    // The entries point at the label, for matching with the wallet statement.
    let refs: Vec<Option<String>> =
        sqlx::query_scalar("SELECT ref FROM ledger_entries WHERE kind = 'courier_charge'")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(refs, [Some("fake-1".into()), Some("fake-1".into())]);

    // Another tick, then the same shipment answered again: the key holds.
    tick(&app).await;
    sqlx::query(
        "UPDATE shipments SET status = 'booking_pending', provider_order_id = NULL, \
                charged_kobo = NULL WHERE order_id = $1",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    app.courier.set_rates(vec![Rate {
        charged_kobo: QUOTE + 1,
        ..default_rate()
    }]);
    tick(&app).await;
    assert_eq!(shipment(&app, f.order_id).await.status, "booked");
    assert_eq!(app.courier.book_calls(), 2);
    assert_eq!(charge_entries(&app, f.order_id).await, booked);
}

/// A free label must not fail the outcome write: the ledger refuses an empty set.
#[sqlx::test]
async fn a_free_label_books_and_posts_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.set_rates(vec![Rate {
        charged_kobo: 0,
        ..default_rate()
    }]);
    let report = tick(&app).await;
    assert_eq!(report.failed, 0, "{report:?}");
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "booked");
    assert_eq!(row.charged_kobo, Some(0));
    assert!(charge_entries(&app, f.order_id).await.is_empty());
}

/// The label outlived the refund: the buyer is repaid in full and the wallet
/// stays debited, so the cost stays on the books.
#[sqlx::test]
async fn refund_with_cancel_failed_leaves_the_cost_on_the_books(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.fake.set_refund_supported(true);
    let f = ready_order(&app).await;
    tick(&app).await;
    app.courier.fail_cancel_with(CourierError::Rejected {
        code: "http_400".into(),
        message: "already processed".into(),
    });
    refund_order(&app.state, f.order_id, true).await.unwrap();
    tick(&app).await;
    assert_eq!(shipment(&app, f.order_id).await.status, "cancel_failed");

    const TOTAL: i64 = 10_000 + QUOTE;
    const FEE: i64 = 10;
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT account::text, sum(amount_kobo)::bigint FROM ledger_entries \
         WHERE order_id = $1 GROUP BY account ORDER BY account::text",
    )
    .bind(f.order_id)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        [
            ("buyer_funds".to_string(), -TOTAL),
            ("courier_costs".to_string(), QUOTE),
            ("escrow".to_string(), 0),
            ("platform_revenue".to_string(), -QUOTE - FEE),
            ("provider_fees".to_string(), FEE),
            ("refunds".to_string(), TOTAL),
        ]
    );
    // No credit of any kind until the sandbox shows what the wallet gets back.
    assert_eq!(charge_entries(&app, f.order_id).await.len(), 2);
}

/// A late payment revives an order after another order took its courier quote:
/// both hold one request token, so the owner is told.
#[sqlx::test]
async fn late_payment_on_a_reused_quote_raises_attention(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let listing = app
        .make_active_store_with_product(SELLER, "Mama Put", 10_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, listing.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let courier_option = |json: &Value| -> Uuid {
        json["stores"][0]["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["method"] == "courier")
            .expect("a courier option")["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    };
    let opts = app.delivery_options(&buyer, address).await;
    let option = courier_option(&opts.json);
    let first = app
        .checkout_with(&buyer, METHOD, address, &[(listing.store_id, option)])
        .await;
    assert_eq!(first.status, 201, "{}", first.json);
    let checkout_a: Uuid = first.json["checkout_id"].as_str().unwrap().parse().unwrap();
    let reference = first.json["reference"].as_str().unwrap().to_string();
    let total = first.json["total_kobo"].as_i64().unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    assert!(
        server::checkout::expire_and_restore(&mut tx, checkout_a)
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();

    // The quote is free again, and a second checkout takes it.
    let opts = app.delivery_options(&buyer, address).await;
    assert_eq!(courier_option(&opts.json), option);
    let second = app
        .checkout_with(&buyer, METHOD, address, &[(listing.store_id, option)])
        .await;
    assert_eq!(second.status, 201, "{}", second.json);

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
    assert_eq!(audits(&app, "order.quote_reused").await.len(), 1);
    let order_id: String = sqlx::query_scalar("SELECT id::text FROM orders WHERE checkout_id = $1")
        .bind(checkout_a)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    let items = json["orders_attention"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{json}");
    assert_eq!(items[0]["attention"], "quote_reused");
    assert_eq!(items[0]["order_id"], order_id);
}

/// A late payment with nobody else on the quote raises nothing.
#[sqlx::test]
async fn late_payment_alone_on_its_quote_raises_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let listing = app
        .make_active_store_with_product(SELLER, "Mama Put", 10_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, listing.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let opts = app.delivery_options(&buyer, address).await;
    let option: Uuid = opts.json["stores"][0]["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "courier")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let res = app
        .checkout_with(&buyer, METHOD, address, &[(listing.store_id, option)])
        .await;
    let checkout: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
    let reference = res.json["reference"].as_str().unwrap().to_string();
    let total = res.json["total_kobo"].as_i64().unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    server::checkout::expire_and_restore(&mut tx, checkout)
        .await
        .unwrap();
    tx.commit().await.unwrap();
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
    assert!(audits(&app, "order.quote_reused").await.is_empty());
}

async fn courier_phone(app: &TestApp, order_id: Uuid) -> Option<Vec<u8>> {
    sqlx::query_scalar("SELECT courier_phone_enc FROM shipments WHERE order_id = $1")
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn refund_clears_the_courier_phone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.set_booking_phone(Some(COURIER_PHONE.into()));
    tick(&app).await;
    assert!(courier_phone(&app, f.order_id).await.is_some());
    refund_order(&app.state, f.order_id, true).await.unwrap();
    assert_eq!(courier_phone(&app, f.order_id).await, None);
}

#[sqlx::test]
async fn booking_that_lands_after_the_refund_keeps_no_phone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.set_booking_phone(Some(COURIER_PHONE.into()));
    app.courier.delay_next_book(Duration::from_millis(300));
    let running = {
        let state = app.state.clone();
        tokio::spawn(async move { run_due_jobs(&state).await })
    };
    book_started(&app, 1).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    running.await.unwrap();
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.provider_order_id.as_deref(), Some("fake-1"));
    assert_eq!(courier_phone(&app, f.order_id).await, None);
}

#[sqlx::test]
async fn blank_courier_name_is_stored_as_none(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.set_rates(vec![Rate {
        courier_name: "  ".into(),
        ..default_rate()
    }]);
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
    let name: Option<String> =
        sqlx::query_scalar("SELECT courier_name FROM shipments WHERE order_id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(name, None);
}

#[sqlx::test]
async fn booking_stores_only_a_safe_tracking_url(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier
        .set_booking_url(&format!("https://track.fake.test/{}", "x".repeat(4096)));
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
    let url: Option<String> =
        sqlx::query_scalar("SELECT tracking_url FROM shipments WHERE order_id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(url, None);
}

#[sqlx::test]
async fn unavailable_cancel_retries_then_succeeds(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    tick(&app).await;
    app.courier.fail_cancel_with(CourierError::Unavailable);
    refund_order(&app.state, f.order_id, true).await.unwrap();

    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "cancel_pending");
    assert_eq!(row.last_error.as_deref(), Some("unavailable"));
    let first = wait_secs(&app, f.order_id).await;
    assert!((290.0..=361.0).contains(&first), "first wait {first}");
    // Not due yet.
    tick(&app).await;
    assert_eq!(app.courier.cancel_calls(), 1);

    app.courier.clear_cancel_failure();
    make_due(&app).await;
    assert_eq!(tick(&app).await.shipments_cancelled, 1);
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "cancelled");
    assert_eq!(row.last_error, None);
    assert_eq!(app.courier.cancel_calls(), 2);
}

#[sqlx::test]
async fn unavailable_cancel_fails_after_8(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    tick(&app).await;
    app.courier.fail_cancel_with(CourierError::Unavailable);
    refund_order(&app.state, f.order_id, true).await.unwrap();
    for _ in 0..7 {
        tick(&app).await;
        assert_eq!(status(&app, f.order_id).await, "cancel_pending");
        assert!(wait_secs(&app, f.order_id).await <= 21_600.0);
        make_due(&app).await;
    }
    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "cancel_failed");
    assert_eq!(row.last_error.as_deref(), Some("unavailable"));
    make_due(&app).await;
    tick(&app).await;
    assert_eq!(app.courier.cancel_calls(), 8);
}

async fn uncertain_cancel_stops(pool: PgPool, err: CourierError, label: &str) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    tick(&app).await;
    app.courier.fail_cancel_with(err);
    refund_order(&app.state, f.order_id, true).await.unwrap();
    tick(&app).await;
    make_due(&app).await;
    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "cancel_failed");
    assert_eq!(row.last_error.as_deref(), Some(label));
    assert_eq!(app.courier.cancel_calls(), 1);
}

/// A cancel that may have landed is not repeated.
#[sqlx::test]
async fn timed_out_cancel_is_terminal(pool: PgPool) {
    uncertain_cancel_stops(pool, CourierError::Timeout, "timed out").await;
}

#[sqlx::test]
async fn inconclusive_cancel_is_terminal(pool: PgPool) {
    uncertain_cancel_stops(pool, CourierError::Inconclusive, "inconclusive").await;
}

#[sqlx::test]
async fn lost_cancel_fence_is_audited(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    tick(&app).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    app.courier.delay_next_cancel(Duration::from_millis(400));
    let running = {
        let state = app.state.clone();
        tokio::spawn(async move { run_due_jobs(&state).await })
    };
    cancel_started(&app, 1).await;
    // Another worker took the row after the lease ran out.
    sqlx::query(
        "UPDATE shipments SET sending_until = now() + interval '2 minutes' WHERE order_id = $1",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    running.await.unwrap();
    assert_eq!(status(&app, f.order_id).await, "cancel_pending");
    let late = audits(&app, "shipment.late_cancel_outcome").await;
    assert_eq!(late.len(), 1);
    assert_eq!(late[0]["outcome"], "cancelled");
}

#[sqlx::test]
async fn old_cancel_pending_is_on_the_attention_list(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    tick(&app).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    let owner = app.owner_token().await;

    let json = attention(&app, &owner).await;
    assert!(
        json["shipments_attention"].as_array().unwrap().is_empty(),
        "{json}"
    );

    sqlx::query(
        "UPDATE shipments SET cancel_requested_at = now() - interval '61 minutes' \
         WHERE order_id = $1",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let json = attention(&app, &owner).await;
    assert_eq!(
        json["shipments_attention"][0]["status"], "cancel_pending",
        "{json}"
    );
}

#[sqlx::test]
async fn refund_cancels_booked_shipment_or_flags_it(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");

    // The cancel goes through the outbox, after commit.
    refund_order(&app.state, f.order_id, true).await.unwrap();
    assert_eq!(status(&app, f.order_id).await, "cancel_pending");
    assert_eq!(app.courier.cancel_calls(), 0);

    assert_eq!(tick(&app).await.shipments_cancelled, 1);
    assert_eq!(status(&app, f.order_id).await, "cancelled");
    assert_eq!(app.courier.cancel_calls(), 1);
    tick(&app).await;
    assert_eq!(app.courier.cancel_calls(), 1);
    assert_eq!(audits(&app, "shipment.cancelled").await.len(), 1);
}

#[sqlx::test]
async fn refused_cancel_flags_the_shipment_and_keeps_the_refund(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.fake.set_refund_supported(true);
    let f = ready_order(&app).await;
    tick(&app).await;
    app.courier.fail_cancel_with(CourierError::Rejected {
        code: "http_400".into(),
        message: "already processed".into(),
    });

    let refund = refund_order(&app.state, f.order_id, true).await.unwrap();
    assert_eq!(refund.status, "processing");
    tick(&app).await;
    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "cancel_failed");
    assert_eq!(row.last_error.as_deref(), Some("http_400"));
    let order: String = sqlx::query_scalar("SELECT status::text FROM orders WHERE id = $1")
        .bind(f.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(order, "refunded");
    assert_eq!(audits(&app, "shipment.cancel_failed").await.len(), 1);

    // Not retried: the owner decides.
    tick(&app).await;
    assert_eq!(app.courier.cancel_calls(), 1);
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    assert_eq!(json["shipments_attention"][0]["status"], "cancel_failed");
    assert!(!json.to_string().contains("already processed"));
}

/// The outbox survives a crash between the refund's commit and the cancel, and
/// a cancel that died mid-call is picked up once its lease has run out.
#[sqlx::test]
async fn cancel_outbox_survives_a_dead_worker(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    tick(&app).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    sqlx::query(
        "UPDATE shipments SET sending_until = now() + interval '1 minute' WHERE order_id = $1",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    tick(&app).await;
    assert_eq!(app.courier.cancel_calls(), 0);

    sqlx::query(
        "UPDATE shipments SET sending_until = now() - interval '1 second' WHERE order_id = $1",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    tick(&app).await;
    assert_eq!(app.courier.cancel_calls(), 1);
    assert_eq!(status(&app, f.order_id).await, "cancelled");
}

#[sqlx::test]
async fn refund_before_booking_cancels_without_a_provider_call(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    assert_eq!(status(&app, f.order_id).await, "cancelled");
    make_due(&app).await;
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "cancelled");
    assert_eq!(app.courier.book_calls(), 0);
    assert_eq!(app.courier.cancel_calls(), 0);
}

#[sqlx::test]
async fn refund_with_unknown_booking_raises_attention(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.fake.set_refund_supported(true);
    let f = ready_order(&app).await;
    app.courier.fail_book_with(CourierError::Timeout);
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booking_unknown");

    let refund = refund_order(&app.state, f.order_id, true).await.unwrap();
    assert_eq!(refund.status, "processing");
    assert_eq!(status(&app, f.order_id).await, "booking_unknown");
    assert_eq!(app.courier.cancel_calls(), 0);
    let alerts = audits(&app, "shipment.live_on_refund").await;
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0]["status"], "booking_unknown");

    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    let items = json["shipments_attention"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{json}");
    assert_eq!(items[0]["status"], "booking_unknown");
    assert_eq!(items[0]["order_status"], "refunded");
}

#[sqlx::test]
async fn refund_with_booking_in_flight_shows_on_the_attention_list(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.fake.set_refund_supported(true);
    let f = ready_order(&app).await;
    hold_lease(&app, f.order_id).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    assert_eq!(status(&app, f.order_id).await, "booking");
    assert_eq!(audits(&app, "shipment.live_on_refund").await.len(), 1);
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    assert_eq!(json["shipments_attention"][0]["status"], "booking");
    assert_eq!(json["shipments_attention"][0]["order_status"], "refunded");
}

/// The POST lands after the refund commits: the fresh label must be cancelled,
/// not left live and unnoticed on a refunded order.
#[sqlx::test]
async fn booking_that_lands_after_the_refund_is_cancelled(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.delay_next_book(Duration::from_millis(300));
    let running = {
        let state = app.state.clone();
        tokio::spawn(async move { run_due_jobs(&state).await })
    };
    book_started(&app, 1).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    running.await.unwrap();
    tick(&app).await;

    let row = shipment(&app, f.order_id).await;
    assert_eq!(row.status, "cancelled");
    assert_eq!(row.provider_order_id.as_deref(), Some("fake-1"));
    assert_eq!(app.courier.cancel_calls(), 1);
}

/// A failed booking answer on an order refunded mid-call needs nobody: no label
/// exists and the buyer is paid back.
#[sqlx::test]
async fn unavailable_booking_of_a_refunded_order_is_cancelled(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.fail_book_with(CourierError::Unavailable);
    app.courier.delay_next_book(Duration::from_millis(300));
    let running = {
        let state = app.state.clone();
        tokio::spawn(async move { run_due_jobs(&state).await })
    };
    book_started(&app, 1).await;
    refund_order(&app.state, f.order_id, true).await.unwrap();
    running.await.unwrap();

    assert_eq!(status(&app, f.order_id).await, "cancelled");
    assert_eq!(app.courier.cancel_calls(), 0);
}

#[sqlx::test]
async fn failed_booking_of_a_refunded_order_leaves_the_attention_list(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.fail_book_with(CourierError::Rejected {
        code: "http_422".into(),
        message: "no".into(),
    });
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booking_failed");
    refund_order(&app.state, f.order_id, true).await.unwrap();
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    assert!(
        json["shipments_attention"].as_array().unwrap().is_empty(),
        "{json}"
    );
}

#[sqlx::test]
async fn held_order_not_booked(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    sqlx::query("UPDATE orders SET held_at = now(), hold_reason = 'x' WHERE id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booking_pending");
    assert_eq!(app.courier.book_calls(), 0);

    sqlx::query("UPDATE orders SET held_at = NULL, hold_reason = NULL WHERE id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");
}

#[sqlx::test]
async fn flagged_checkout_not_booked(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    sqlx::query("UPDATE checkouts SET provider_flagged = true WHERE id = $1")
        .bind(f.checkout_id)
        .execute(&app.pool)
        .await
        .unwrap();
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booking_pending");
    assert_eq!(app.courier.book_calls(), 0);
}

#[sqlx::test]
async fn courier_phone_is_encrypted_at_rest(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.set_booking_phone(Some(COURIER_PHONE.into()));
    tick(&app).await;
    let enc: Vec<u8> =
        sqlx::query_scalar("SELECT courier_phone_enc FROM shipments WHERE order_id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(!String::from_utf8_lossy(&enc).contains(COURIER_PHONE));
    assert_eq!(app.state.cipher.decrypt(&enc).unwrap(), COURIER_PHONE);
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE detail::text LIKE '%' || $1 || '%'",
    )
    .bind(COURIER_PHONE)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0);
}

#[sqlx::test]
async fn shipments_appear_on_attention_list_without_phone_for_support(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.courier.set_booking_phone(Some(COURIER_PHONE.into()));
    app.courier.set_rates(vec![Rate {
        charged_kobo: 200_000,
        ..default_rate()
    }]);
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booked");

    // A healthy booking would not be listed; this one cost more than the quote.
    let support = app.make_support().await;
    let json = attention(&app, &support).await;
    let items = json["shipments_attention"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{json}");
    assert_eq!(items[0]["order_id"], f.order_id.to_string());
    assert_eq!(items[0]["store_id"], f.store_id.to_string());
    assert!(items[0].get("courier_phone").is_none());
    assert!(!json.to_string().contains(COURIER_PHONE), "{json}");

    // Each kind is listed.
    for kind in ["booking_unknown", "booking_failed", "cancel_failed"] {
        sqlx::query("UPDATE shipments SET status = $2::shipment_status WHERE order_id = $1")
            .bind(f.order_id)
            .bind(kind)
            .execute(&app.pool)
            .await
            .unwrap();
        let json = attention(&app, &support).await;
        assert_eq!(json["shipments_attention"][0]["status"], kind, "{json}");
    }
}

#[sqlx::test]
async fn healthy_shipment_is_not_on_the_attention_list(pool: PgPool) {
    let app = TestApp::new(pool).await;
    ready_order(&app).await;
    tick(&app).await;
    let owner = app.owner_token().await;
    let json = attention(&app, &owner).await;
    assert!(json["shipments_attention"].as_array().unwrap().is_empty());
    assert!(json["wallet_alerts"].as_array().unwrap().is_empty());
}

#[sqlx::test]
async fn no_courier_configured_books_nothing_but_still_reaps(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let f = ready_order(&app).await;
    app.state.courier = None;
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booking_pending");

    sqlx::query(
        "UPDATE shipments SET status = 'booking', sending_until = now() - interval '1 second' \
         WHERE order_id = $1",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    tick(&app).await;
    assert_eq!(status(&app, f.order_id).await, "booking_unknown");
}
