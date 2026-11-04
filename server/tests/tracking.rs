//! `GET /orders/{id}/tracking`: the fixed timeline, contact details and the
//! handover code, and the seller's time-limited view of the buyer's contact.

mod common;

use axum::http::Method;
use common::{BUYER_ADDRESS_LINE, TestApp};
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    payments::PaymentStatus,
};
use sqlx::PgPool;
use uuid::Uuid;

const METHOD: &str = "mono_directpay";
const SELLER: &str = "08011111111";
const SELLER2: &str = "08011111112";
const BUYER: &str = "08022222221";
const BUYER_PHONE: &str = "+2348077777777";
const STORE_PHONE_E164: &str = "+2348031234567";
const COURIER_PHONE: &str = "08055501234";

struct Fixture {
    order_id: Uuid,
    seller: String,
    buyer: String,
}

async fn pay(app: &TestApp, res: common::TestResponse) -> Uuid {
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

/// One paid order from `seller_phone`'s store, bought by `buyer_phone`, "self" or "courier".
async fn paid_order_for(
    app: &TestApp,
    seller_phone: &str,
    store: &str,
    buyer_phone: &str,
    method: &str,
) -> Fixture {
    let listing = app
        .make_active_store_with_product(seller_phone, store, 10_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(buyer_phone).await;
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
        buyer,
    }
}

async fn paid_order(app: &TestApp, method: &str) -> Fixture {
    paid_order_for(app, SELLER, "Mama Put", BUYER, method).await
}

async fn track(app: &TestApp, id: impl std::fmt::Display, token: &str) -> common::TestResponse {
    app.request(
        Method::GET,
        &format!("/orders/{id}/tracking"),
        None,
        Some(token),
    )
    .await
}

async fn step(app: &TestApp, f: &Fixture, name: &str) {
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/{name}", f.order_id),
            None,
            Some(&f.seller),
        )
        .await;
    assert_eq!(res.status, 200, "{name}: {}", res.json);
}

/// A self-delivery order, moved to `out_for_delivery` through the real steps.
async fn out_for_delivery(app: &TestApp) -> Fixture {
    let f = paid_order(app, "self").await;
    step(app, &f, "picked-up").await;
    step(app, &f, "out-for-delivery").await;
    f
}

async fn code_of(app: &TestApp, id: Uuid) -> String {
    let enc: Vec<u8> = sqlx::query_scalar("SELECT handover_code_enc FROM orders WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    app.state.cipher.decrypt(&enc).unwrap()
}

async fn reveal_rows(app: &TestApp, id: Uuid) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = 'order.contact_revealed' AND subject_id = $1",
    )
    .bind(id)
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

fn step_of<'a>(json: &'a Value, key: &str) -> &'a Value {
    json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("no step {key}"))
}

fn current_of(json: &Value) -> Vec<&str> {
    json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["current"] == true)
        .map(|s| s["key"].as_str().unwrap())
        .collect()
}

fn no_store(res: &common::TestResponse) -> bool {
    res.headers
        .get("cache-control")
        .is_some_and(|v| v == "no-store")
}

/// Installs a live shipment for a courier order.
async fn book(app: &TestApp, id: Uuid, url: Option<&str>) {
    let enc = app.state.cipher.encrypt(COURIER_PHONE);
    sqlx::query(
        "INSERT INTO shipments (order_id, provider, status, provider_order_id, tracking_url, \
                                courier_name, courier_phone_enc, quoted_kobo) \
         VALUES ($1, 'fake', 'booked', 'SB-1', $2, 'Swift Riders', $3, 150000)",
    )
    .bind(id)
    .bind(url)
    .bind(enc)
    .execute(&app.pool)
    .await
    .unwrap();
}

async fn set_status(app: &TestApp, id: Uuid, status: &str) {
    sqlx::query("UPDATE orders SET status = $2::order_status WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(&app.pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn tracking_steps_fill_in_as_order_moves(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;

    let res = track(&app, f.order_id, &f.buyer).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let keys: Vec<&str> = res.json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "paid",
            "preparing",
            "picked_up",
            "out_for_delivery",
            "delivered",
            "confirmed"
        ]
    );
    assert_eq!(step_of(&res.json, "preparing")["label"], "Seller preparing");
    assert!(step_of(&res.json, "paid")["at"].is_string());
    assert!(step_of(&res.json, "preparing")["at"].is_null());
    assert_eq!(current_of(&res.json), ["preparing"]);
    assert_eq!(res.json["status"], "paid");
    assert!(res.json["promised_date"].is_string());

    step(&app, &f, "picked-up").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(step_of(&res.json, "picked_up")["at"].is_string());
    assert_eq!(current_of(&res.json), ["picked_up"]);

    step(&app, &f, "out-for-delivery").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(step_of(&res.json, "out_for_delivery")["at"].is_string());
    assert_eq!(current_of(&res.json), ["out_for_delivery"]);

    let code = code_of(&app, f.order_id).await;
    let del = app
        .request(
            Method::POST,
            &format!("/orders/{}/delivered", f.order_id),
            Some(json!({ "handover_code": code })),
            Some(&f.seller),
        )
        .await;
    assert_eq!(del.status, 200, "{}", del.json);
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(step_of(&res.json, "delivered")["at"].is_string());
    // Entering the code is the buyer's confirmation, so the order completes.
    assert!(step_of(&res.json, "confirmed")["at"].is_string());
    assert_eq!(current_of(&res.json), ["confirmed"]);
    assert_eq!(res.json["status"], "completed");
}

#[sqlx::test]
async fn courier_preparing_step_takes_ready_at(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "courier").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(step_of(&res.json, "preparing")["at"].is_null());
    step(&app, &f, "ready").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(step_of(&res.json, "preparing")["at"].is_string());
    assert_eq!(current_of(&res.json), ["preparing"]);
}

#[sqlx::test]
async fn handover_code_shown_only_out_for_delivery(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("handover_code").is_none(), "{}", res.json);
    step(&app, &f, "picked-up").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("handover_code").is_none(), "{}", res.json);

    step(&app, &f, "out-for-delivery").await;
    let code = code_of(&app, f.order_id).await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert_eq!(res.json["handover_code"], code);
    assert_eq!(
        res.json["handover_note"],
        "Give this only to the person handing you the parcel"
    );
    // Never to the seller.
    let res = track(&app, f.order_id, &f.seller).await;
    assert!(!res.json.to_string().contains(&code), "{}", res.json);
    assert!(res.json.get("handover_code").is_none());

    set_status(&app, f.order_id, "delivered").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("handover_code").is_none(), "{}", res.json);
}

#[sqlx::test]
async fn seller_sees_buyer_contact_only_out_for_delivery_and_audited(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;

    // Before: the area council only.
    let res = track(&app, f.order_id, &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["delivery_area"], "amac");
    assert!(res.json.get("buyer_contact").is_none());
    let text = res.json.to_string();
    assert!(!text.contains(BUYER_ADDRESS_LINE) && !text.contains(BUYER_PHONE));
    step(&app, &f, "picked-up").await;
    let res = track(&app, f.order_id, &f.seller).await;
    assert!(res.json.get("buyer_contact").is_none(), "{}", res.json);
    assert!(reveal_rows(&app, f.order_id).await.is_empty());

    // During: the full contact, landmark included, and one audit row.
    step(&app, &f, "out-for-delivery").await;
    let res = track(&app, f.order_id, &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(
        res.json["buyer_contact"],
        json!({
            "recipient_name": "Ada Buyer",
            "phone": BUYER_PHONE,
            "address_line": BUYER_ADDRESS_LINE,
            "landmark": "Opposite the blue gate"
        })
    );
    assert!(no_store(&res));
    let rows = reveal_rows(&app, f.order_id).await;
    assert_eq!(rows.len(), 1);
    let detail = rows[0].to_string();
    assert!(!detail.contains(BUYER_PHONE) && !detail.contains(BUYER_ADDRESS_LINE));
    assert!(!detail.contains("Ada"), "{detail}");

    // A second view is a view, not a second reveal.
    let again = track(&app, f.order_id, &f.seller).await;
    assert!(again.json["buyer_contact"].is_object());
    assert_eq!(reveal_rows(&app, f.order_id).await.len(), 1);
    // The seller never gets the buyer-facing contact block.
    assert!(again.json.get("contact").is_none());
}

#[sqlx::test]
async fn courier_order_seller_never_sees_buyer_contact(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "courier").await;
    book(&app, f.order_id, Some("https://track.example/abc")).await;
    for status in ["paid", "picked_up", "out_for_delivery", "delivered"] {
        set_status(&app, f.order_id, status).await;
        let res = track(&app, f.order_id, &f.seller).await;
        assert_eq!(res.status, 200, "{status}: {}", res.json);
        assert!(res.json.get("buyer_contact").is_none(), "{status}");
        assert_eq!(res.json["delivery_area"], "amac");
        let text = res.json.to_string();
        for secret in [
            BUYER_PHONE,
            BUYER_ADDRESS_LINE,
            "Ada Buyer",
            COURIER_PHONE,
            "track.example",
        ] {
            assert!(!text.contains(secret), "{status}: {secret} in {text}");
        }
    }
    assert!(reveal_rows(&app, f.order_id).await.is_empty());
}

#[sqlx::test]
async fn buyer_gets_courier_contact_and_https_link(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "courier").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("contact").is_none(), "no booking yet");

    book(&app, f.order_id, Some("https://track.example/abc")).await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert_eq!(
        res.json["contact"],
        json!({ "name": "Swift Riders", "phone": COURIER_PHONE })
    );
    assert_eq!(res.json["tracking_url"], "https://track.example/abc");
    assert!(res.json.get("buyer_contact").is_none());

    // A link that is not https is never shown.
    sqlx::query("UPDATE shipments SET tracking_url = 'http://track.example/abc'")
        .execute(&app.pool)
        .await
        .unwrap();
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("tracking_url").is_none(), "{}", res.json);
}

#[sqlx::test]
async fn self_delivery_contact_is_the_store_once_out_for_delivery(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("contact").is_none(), "{}", res.json);
    step(&app, &f, "picked-up").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("contact").is_none(), "{}", res.json);
    step(&app, &f, "out-for-delivery").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert_eq!(
        res.json["contact"],
        json!({ "name": "Mama Put", "phone": STORE_PHONE_E164 })
    );
    assert!(res.json.get("tracking_url").is_none());
}

#[sqlx::test]
async fn contact_hidden_after_delivery(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = out_for_delivery(&app).await;
    assert!(
        track(&app, f.order_id, &f.buyer).await.json["contact"].is_object(),
        "visible while out for delivery"
    );
    for status in [
        "delivered",
        "completed",
        "cancelled",
        "refund_pending",
        "refunded",
    ] {
        set_status(&app, f.order_id, status).await;
        let buyer = track(&app, f.order_id, &f.buyer).await;
        assert!(buyer.json.get("contact").is_none(), "{status}");
        let seller = track(&app, f.order_id, &f.seller).await;
        assert!(seller.json.get("buyer_contact").is_none(), "{status}");
        assert_eq!(seller.json["delivery_area"], "amac");
    }

    // A courier order's contact goes at delivery too.
    let g = paid_order_for(&app, SELLER2, "Other Shop", "08022222229", "courier").await;
    book(&app, g.order_id, None).await;
    assert!(track(&app, g.order_id, &g.buyer).await.json["contact"].is_object());
    set_status(&app, g.order_id, "delivered").await;
    let res = track(&app, g.order_id, &g.buyer).await;
    assert!(res.json.get("contact").is_none(), "{}", res.json);
}

/// The courier's hosted page may show the rider: the link goes when the contact does.
#[sqlx::test]
async fn tracking_url_hidden_from_delivery_on(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "courier").await;
    book(&app, f.order_id, Some("https://track.example/abc")).await;
    for status in ["paid", "picked_up", "out_for_delivery"] {
        set_status(&app, f.order_id, status).await;
        let res = track(&app, f.order_id, &f.buyer).await;
        assert!(res.json["tracking_url"].is_string(), "{status}");
    }
    for status in [
        "delivered",
        "completed",
        "cancelled",
        "refund_pending",
        "refunded",
    ] {
        set_status(&app, f.order_id, status).await;
        let res = track(&app, f.order_id, &f.buyer).await;
        assert!(res.json.get("tracking_url").is_none(), "{status}");
    }
}

/// A shipment that is not live shows nothing, though the order is still `paid`.
#[sqlx::test]
async fn nothing_shown_for_a_shipment_that_is_not_live(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "courier").await;
    book(&app, f.order_id, Some("https://track.example/abc")).await;
    for status in ["booking_unknown", "cancel_pending", "cancelled"] {
        sqlx::query("UPDATE shipments SET status = $1::shipment_status")
            .bind(status)
            .execute(&app.pool)
            .await
            .unwrap();
        let res = track(&app, f.order_id, &f.buyer).await;
        assert!(res.json.get("contact").is_none(), "{status}");
        assert!(res.json.get("tracking_url").is_none(), "{status}");
    }
}

#[sqlx::test]
async fn seller_gets_404_until_the_order_is_paid(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let listing = app
        .make_active_store_with_product(SELLER, "Mama Put", 10_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, listing.variant_id, 1).await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);
    let id = res.json["orders"][0]["order_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(track(&app, &id, &listing.seller_token).await.status, 404);
    assert_eq!(track(&app, &id, &buyer).await.status, 200);

    // Cancelled without ever being paid, as an expired checkout leaves it.
    let checkout: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    assert!(
        server::checkout::expire_and_restore(&mut tx, checkout)
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    assert_eq!(track(&app, &id, &listing.seller_token).await.status, 404);
    assert_eq!(track(&app, &id, &buyer).await.status, 200);
}

/// A corrupt pickup address is no reason to fail the buyer's tracking: only the
/// phone is read. A deleted store simply has nothing to call.
#[sqlx::test]
async fn store_contact_reads_only_the_phone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = out_for_delivery(&app).await;
    sqlx::query("UPDATE stores SET pickup_address_enc = '\\x00'::bytea")
        .execute(&app.pool)
        .await
        .unwrap();
    let res = track(&app, f.order_id, &f.buyer).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["contact"]["phone"], STORE_PHONE_E164);
    assert!(res.json["handover_code"].is_string());

    sqlx::query("UPDATE stores SET deleted_at = now()")
        .execute(&app.pool)
        .await
        .unwrap();
    let res = track(&app, f.order_id, &f.buyer).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(res.json.get("contact").is_none(), "{}", res.json);
}

#[sqlx::test]
async fn tracking_private_to_buyer_and_seller(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = out_for_delivery(&app).await;

    // A seller of another store.
    let other = app
        .make_active_store_with_product(SELLER2, "Other Shop", 5_000, 1)
        .await;
    let res = track(&app, f.order_id, &other.seller_token).await;
    assert_eq!(res.status, 404, "{}", res.json);
    // Another buyer.
    let (_, stranger) = app.sign_up_verified("08022222223").await;
    assert_eq!(track(&app, f.order_id, &stranger).await.status, 404);
    // Staff are neither buyer nor seller.
    let owner = app.owner_token().await;
    assert_eq!(track(&app, f.order_id, &owner).await.status, 404);
    let support = app.make_support().await;
    assert_eq!(track(&app, f.order_id, &support).await.status, 404);
    // Malformed and unknown ids.
    assert_eq!(track(&app, "not-a-uuid", &f.buyer).await.status, 404);
    assert_eq!(track(&app, Uuid::new_v4(), &f.buyer).await.status, 404);
    // Signed out.
    let res = app
        .request(
            Method::GET,
            &format!("/orders/{}/tracking", f.order_id),
            None,
            None,
        )
        .await;
    assert_eq!(res.status, 401);
    // None of those wrote a reveal.
    assert!(reveal_rows(&app, f.order_id).await.is_empty());
    // The two parties still can.
    assert_eq!(track(&app, f.order_id, &f.buyer).await.status, 200);
    assert_eq!(track(&app, f.order_id, &f.seller).await.status, 200);
}

#[sqlx::test]
async fn delay_note_is_derived_and_changes_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("delay_note").is_none(), "{}", res.json);

    let lagos_today = "(now() AT TIME ZONE 'Africa/Lagos')::date";
    sqlx::query(&format!(
        "UPDATE orders SET promised_date = {lagos_today} - 1"
    ))
    .execute(&app.pool)
    .await
    .unwrap();
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json["delay_note"].is_string(), "{}", res.json);
    assert_eq!(res.json["status"], "paid");
    assert!(track(&app, f.order_id, &f.seller).await.json["delay_note"].is_string());

    // Promised today is not late.
    sqlx::query(&format!("UPDATE orders SET promised_date = {lagos_today}"))
        .execute(&app.pool)
        .await
        .unwrap();
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("delay_note").is_none(), "{}", res.json);

    // Delivered orders are never late.
    sqlx::query(&format!(
        "UPDATE orders SET promised_date = {lagos_today} - 5"
    ))
    .execute(&app.pool)
    .await
    .unwrap();
    set_status(&app, f.order_id, "delivered").await;
    let res = track(&app, f.order_id, &f.buyer).await;
    assert!(res.json.get("delay_note").is_none(), "{}", res.json);
}

#[sqlx::test]
async fn held_flag_is_for_the_seller_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    assert!(
        track(&app, f.order_id, &f.seller)
            .await
            .json
            .get("held")
            .is_none()
    );
    sqlx::query("UPDATE orders SET held_at = now(), hold_reason = 'test'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(track(&app, f.order_id, &f.seller).await.json["held"], true);
    assert!(
        track(&app, f.order_id, &f.buyer)
            .await
            .json
            .get("held")
            .is_none()
    );
}

/// A hold does not gate the contact: a held order may still be delivered.
#[sqlx::test]
async fn held_order_out_for_delivery_keeps_its_contacts(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = out_for_delivery(&app).await;
    sqlx::query("UPDATE orders SET held_at = now(), hold_reason = 'test'")
        .execute(&app.pool)
        .await
        .unwrap();
    let seller = track(&app, f.order_id, &f.seller).await;
    assert_eq!(seller.json["held"], true);
    assert_eq!(seller.json["buyer_contact"]["phone"], BUYER_PHONE);
    let buyer = track(&app, f.order_id, &f.buyer).await;
    assert!(buyer.json.get("held").is_none());
    assert_eq!(buyer.json["contact"]["phone"], STORE_PHONE_E164);
    assert!(buyer.json["handover_code"].is_string());
}

/// The reason for the advisory lock: first views at once write one row.
#[sqlx::test]
async fn concurrent_first_views_audit_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = out_for_delivery(&app).await;
    let view = || track(&app, f.order_id, &f.buyer);
    let (a, b, c, d) = tokio::join!(view(), view(), view(), view());
    for res in [a, b, c, d] {
        assert_eq!(res.status, 200, "{}", res.json);
    }
    assert_eq!(reveal_rows(&app, f.order_id).await.len(), 1);
}

#[sqlx::test]
async fn buyer_view_audits_contact_once_and_sends_no_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = out_for_delivery(&app).await;
    let first = track(&app, f.order_id, &f.buyer).await;
    assert!(no_store(&first));
    track(&app, f.order_id, &f.buyer).await;
    let rows = reveal_rows(&app, f.order_id).await;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].to_string().contains(STORE_PHONE_E164));
}
