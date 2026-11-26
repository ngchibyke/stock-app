//! Collect from a branch (part 7e-1, task 10): a `collect` option per branch group
//! where the branch has it on, fee 0; the branch marks the order ready and hands it
//! over against the buyer's code; an order left uncollected past the window is
//! refunded in full, its stock goes back to the branch and the seller gets no strike.

mod common;

use axum::http::Method;
use common::{BUYER_ADDRESS_LINE, Listing, STORE_PHONE, TestApp, TestResponse, group_option};
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    delivery::collect,
    escrow::jobs::run_due_jobs,
    payments::PaymentStatus,
};
use sqlx::PgPool;
use uuid::Uuid;

const METHOD: &str = "mono_directpay";
/// What the store's pickup address is, in `make_active_store_with_product`.
const PICKUP_ADDRESS: &str = "12 Unique Street, Wuse 2";
/// The buyer's saved address phone, in `add_address`.
const BUYER_ADDRESS_PHONE: &str = "8077777777";

struct Fx {
    listing: Listing,
    branch: Uuid,
    order_id: Uuid,
    buyer_id: Uuid,
    buyer: String,
    buyer_phone: String,
}

async fn first_branch(app: &TestApp, store: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM store_branches WHERE store_id = $1 AND position = 0")
        .bind(store)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn set_collect(app: &TestApp, branch: Uuid, on: bool) {
    sqlx::query("UPDATE store_branches SET collect_enabled = $2 WHERE id = $1")
        .bind(branch)
        .bind(on)
        .execute(&app.pool)
        .await
        .unwrap();
}

fn methods(json: &Value) -> Vec<String> {
    common::first_group_options(json)
        .iter()
        .map(|o| o["method"].as_str().unwrap().to_string())
        .collect()
}

/// Pays the checkout answered by `res`; returns the order id.
async fn pay(app: &TestApp, res: &TestResponse) -> Uuid {
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

/// A store whose first branch offers collect, and a buyer with one unit in the
/// cart and its collect option chosen at checkout. `n` keeps phones distinct.
async fn collect_checkout(app: &TestApp, n: u32) -> (Fx, TestResponse) {
    let listing = app
        .make_active_store_with_product(
            &format!("0806{:07}", 1_000 + n),
            &format!("Counter {n}"),
            10_000,
            5,
        )
        .await;
    let branch = first_branch(app, listing.store_id).await;
    set_collect(app, branch, true).await;
    let buyer_phone = format!("0807{:07}", 1_000 + n);
    let (buyer_id, buyer) = app.sign_up_verified(&buyer_phone).await;
    assert_eq!(
        app.set_cart(&buyer, listing.variant_id, 1).await.status,
        200
    );
    let address = app.buyer_address(&buyer).await;
    let opts = app.delivery_options(&buyer, address).await;
    assert_eq!(opts.status, 200, "{}", opts.json);
    let choice = group_option(&opts.json, "collect");
    let res = app
        .checkout_with(&buyer, METHOD, address, std::slice::from_ref(&choice))
        .await;
    (
        Fx {
            listing,
            branch,
            order_id: Uuid::nil(),
            buyer_id,
            buyer,
            buyer_phone,
        },
        res,
    )
}

async fn paid_collect(app: &TestApp, n: u32) -> Fx {
    let (mut fx, res) = collect_checkout(app, n).await;
    fx.order_id = pay(app, &res).await;
    fx
}

async fn step(app: &TestApp, id: Uuid, name: &str, token: &str) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/orders/{id}/{name}"),
        None,
        Some(token),
    )
    .await
}

async fn deliver(app: &TestApp, id: Uuid, code: &str, token: &str) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/orders/{id}/delivered"),
        Some(json!({ "handover_code": code })),
        Some(token),
    )
    .await
}

async fn tracking(app: &TestApp, id: Uuid, token: &str) -> TestResponse {
    app.request(
        Method::GET,
        &format!("/orders/{id}/tracking"),
        None,
        Some(token),
    )
    .await
}

async fn status_of(app: &TestApp, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM orders WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn code_of(app: &TestApp, id: Uuid) -> String {
    let enc: Vec<u8> = sqlx::query_scalar("SELECT handover_code_enc FROM orders WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    app.state.cipher.decrypt(&enc).unwrap()
}

fn wrong(code: &str) -> String {
    if code == "000000" {
        "111111".into()
    } else {
        "000000".into()
    }
}

/// Moves a ready order's `ready_at` to `age` (an SQL interval) ago, and its stored
/// `collect_by` with it.
async fn age_ready(app: &TestApp, order: Uuid, age: &str) {
    sqlx::query(&format!(
        "UPDATE orders SET ready_at = now() - interval '{age}', \
                collect_by = now() - interval '{age}' + (collect_by - ready_at) \
         WHERE id = $1"
    ))
    .bind(order)
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Marks the order ready and moves `ready_at` back by `age` (an SQL interval).
async fn ready_since(app: &TestApp, fx: &Fx, age: &str) {
    let res = step(app, fx.order_id, "ready", &fx.listing.seller_token).await;
    assert_eq!(res.status, 200, "{}", res.json);
    age_ready(app, fx.order_id, age).await;
}

async fn notices(app: &TestApp, user: Uuid, kind: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1 AND kind = $2")
        .bind(user)
        .bind(kind)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn audits(app: &TestApp, order: Uuid, action: &str) -> Vec<(Option<Uuid>, Value)> {
    sqlx::query_as(
        "SELECT actor_id, detail FROM audit_log \
         WHERE subject_type = 'order' AND subject_id = $1 AND action = $2 ORDER BY id",
    )
    .bind(order)
    .bind(action)
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

async fn branch_counts(app: &TestApp, branch: Uuid, variant: Uuid) -> (i32, i32) {
    sqlx::query_as(
        "SELECT stock, reserved_quantity FROM branch_stock WHERE branch_id = $1 AND variant_id = $2",
    )
    .bind(branch)
    .bind(variant)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// A staff member tied to `branch` of `store`; returns their token.
async fn staff_at(
    app: &TestApp,
    phone: &str,
    store: Uuid,
    branch: Uuid,
    owner: Uuid,
) -> (Uuid, String) {
    let (id, token) = app.sign_up_verified(phone).await;
    sqlx::query(
        "INSERT INTO store_members (store_id, user_id, role, branch_id, added_by) \
         VALUES ($1, $2, 'staff', $3, $4)",
    )
    .bind(store)
    .bind(id)
    .bind(branch)
    .bind(owner)
    .execute(&app.pool)
    .await
    .unwrap();
    (id, token)
}

/// A second branch of `store`, so staff can be tied elsewhere.
async fn other_branch(app: &TestApp, store: Uuid) -> Uuid {
    let addr = app.state.cipher.encrypt("9 Other Road, Garki");
    sqlx::query_scalar(
        "INSERT INTO store_branches (store_id, name, pickup_address_enc, position) \
         VALUES ($1, 'Garki', $2, 1) RETURNING id",
    )
    .bind(store)
    .bind(addr)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn collect_offered_only_when_branch_enabled(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let listing = app
        .make_active_store_with_product("08061000001", "Counter", 10_000, 5)
        .await;
    let branch = first_branch(&app, listing.store_id).await;
    let (_, buyer) = app.sign_up_verified("08071000001").await;
    assert_eq!(
        app.set_cart(&buyer, listing.variant_id, 1).await.status,
        200
    );
    let address = app.buyer_address(&buyer).await;

    // Off by default.
    let res = app.delivery_options(&buyer, address).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(
        !methods(&res.json).contains(&"collect".to_string()),
        "{}",
        res.json
    );

    set_collect(&app, branch, true).await;
    let res = app.delivery_options(&buyer, address).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(
        methods(&res.json).contains(&"collect".to_string()),
        "{}",
        res.json
    );
    let choice = group_option(&res.json, "collect");
    // A repeat call reuses the unused collect option instead of adding one.
    let again = app.delivery_options(&buyer, address).await;
    assert_eq!(group_option(&again.json, "collect"), choice);

    // Switched off after the quote: checkout refuses the option.
    set_collect(&app, branch, false).await;
    let res = app
        .checkout_with(&buyer, METHOD, address, std::slice::from_ref(&choice))
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "delivery_option_invalid");
}

#[sqlx::test]
async fn collect_fee_zero(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (fx, res) = collect_checkout(&app, 1).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["total_kobo"], 10_000, "{}", res.json);
    let opts = app
        .delivery_options(&fx.buyer, app.buyer_address(&fx.buyer).await)
        .await;
    let collect = common::first_group_options(&opts.json)
        .iter()
        .find(|o| o["method"] == "collect")
        .cloned()
        .unwrap();
    assert_eq!(collect["fee_kobo"], 0);
    assert!(collect["promised_date"].is_string());

    let order_id = pay(&app, &res).await;
    let (method, fee, promised, address): (String, i64, Option<time::Date>, Option<Vec<u8>>) =
        sqlx::query_as(
            "SELECT delivery_method::text, delivery_fee_kobo, promised_date, delivery_address_enc \
             FROM orders WHERE id = $1",
        )
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!((method.as_str(), fee), ("collect", 0));
    assert!(promised.is_some());
    // Data minimisation: a collect order keeps no copy of the buyer's address.
    assert!(address.is_none());
}

#[sqlx::test]
async fn address_and_phone_shown_to_buyer_only_after_payment(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (mut fx, res) = collect_checkout(&app, 2).await;
    assert_eq!(res.status, 201, "{}", res.json);

    // Before paying: the options show the branch's name and area, no address or phone.
    let opts = app
        .delivery_options(&fx.buyer, app.buyer_address(&fx.buyer).await)
        .await;
    let text = opts.json.to_string();
    assert!(!text.contains(PICKUP_ADDRESS), "{text}");
    assert!(!text.contains(&STORE_PHONE[1..]), "{text}");
    let checkout_id: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
    let unpaid: Uuid = sqlx::query_scalar("SELECT id FROM orders WHERE checkout_id = $1")
        .bind(checkout_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let t = tracking(&app, unpaid, &fx.buyer).await;
    assert_eq!(t.status, 200, "{}", t.json);
    assert!(t.json.get("pickup").is_none(), "{}", t.json);

    // After paying: the branch's address and business phone.
    fx.order_id = pay(&app, &res).await;
    let t = tracking(&app, fx.order_id, &fx.buyer).await;
    assert_eq!(t.status, 200, "{}", t.json);
    assert_eq!(t.json["pickup"]["address"], PICKUP_ADDRESS, "{}", t.json);
    assert!(
        t.json["pickup"]["phone"]
            .as_str()
            .unwrap()
            .contains(&STORE_PHONE[1..]),
        "{}",
        t.json
    );
    assert!(t.json["pickup"]["branch_name"].is_string(), "{}", t.json);
    // No code before the order is ready.
    assert!(t.json.get("handover_code").is_none(), "{}", t.json);
    // The reveal is audited once per viewer, with no value in it.
    let rows = audits(&app, fx.order_id, "order.contact_revealed").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, json!({ "kind": "pickup_point" }));
    tracking(&app, fx.order_id, &fx.buyer).await;
    assert_eq!(
        audits(&app, fx.order_id, "order.contact_revealed")
            .await
            .len(),
        1
    );

    // Not after collection.
    let res = step(&app, fx.order_id, "ready", &fx.listing.seller_token).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let code = code_of(&app, fx.order_id).await;
    let res = deliver(&app, fx.order_id, &code, &fx.listing.seller_token).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let t = tracking(&app, fx.order_id, &fx.buyer).await;
    assert!(t.json.get("pickup").is_none(), "{}", t.json);
}

#[sqlx::test]
async fn branch_never_sees_buyer_phone_on_collect(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let fx = paid_collect(&app, 3).await;
    let (_, staff) = staff_at(
        &app,
        "08091000003",
        fx.listing.store_id,
        fx.branch,
        fx.listing.seller_id,
    )
    .await;
    let elsewhere = other_branch(&app, fx.listing.store_id).await;
    let (_, stranger) = staff_at(
        &app,
        "08091000004",
        fx.listing.store_id,
        elsewhere,
        fx.listing.seller_id,
    )
    .await;
    let res = step(&app, fx.order_id, "ready", &staff).await;
    assert_eq!(res.status, 200, "{}", res.json);

    let buyer_digits = &fx.buyer_phone[1..];
    for token in [&fx.listing.seller_token, &staff] {
        for path in [
            format!("/orders/{}", fx.order_id),
            format!("/orders/{}/tracking", fx.order_id),
            format!("/stores/{}/orders", fx.listing.store_id),
        ] {
            let res = app.request(Method::GET, &path, None, Some(token)).await;
            assert_eq!(res.status, 200, "{path}: {}", res.json);
            let text = res.json.to_string();
            assert!(!text.contains(buyer_digits), "{path}: {text}");
            assert!(!text.contains(BUYER_ADDRESS_PHONE), "{path}: {text}");
            assert!(!text.contains(BUYER_ADDRESS_LINE), "{path}: {text}");
            assert!(res.json.get("buyer_contact").is_none(), "{path}: {text}");
            assert!(res.json.get("handover_code").is_none(), "{path}: {text}");
        }
        // The buyer's name, never the phone.
        let res = app
            .request(
                Method::GET,
                &format!("/orders/{}", fx.order_id),
                None,
                Some(token),
            )
            .await;
        assert!(res.json["buyer_display_name"].is_string(), "{}", res.json);
    }
    // Staff of another branch cannot see or act on it.
    let res = tracking(&app, fx.order_id, &stranger).await;
    assert_eq!(res.status, 404);
    let res = deliver(&app, fx.order_id, "123456", &stranger).await;
    assert_eq!(res.status, 404);
}

#[sqlx::test]
async fn ready_then_collected_with_code(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let fx = paid_collect(&app, 4).await;
    let seller = &fx.listing.seller_token;
    let (staff_id, staff) = staff_at(
        &app,
        "08091000005",
        fx.listing.store_id,
        fx.branch,
        fx.listing.seller_id,
    )
    .await;

    // Delivery steps are not for collect orders, nor is a code before ready.
    for name in ["picked-up", "out-for-delivery"] {
        let res = step(&app, fx.order_id, name, seller).await;
        assert_eq!(res.status, 409, "{name}: {}", res.json);
    }
    let res = deliver(&app, fx.order_id, "123456", seller).await;
    assert_eq!(res.status, 409, "{}", res.json);
    // The buyer cannot mark it ready.
    let res = step(&app, fx.order_id, "ready", &fx.buyer).await;
    assert_eq!(res.status, 404);

    let res = step(&app, fx.order_id, "ready", &staff).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "paid");
    assert_eq!(
        notices(&app, fx.buyer_id, "order_ready_for_collection").await,
        1
    );
    let code = code_of(&app, fx.order_id).await;
    // Repeating it changes nothing: same code, one notice, one audit row.
    let res = step(&app, fx.order_id, "ready", seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(code_of(&app, fx.order_id).await, code);
    assert_eq!(
        notices(&app, fx.buyer_id, "order_ready_for_collection").await,
        1
    );
    assert_eq!(audits(&app, fx.order_id, "order.ready").await.len(), 1);
    // No courier shipment for a collect order.
    let shipments: i64 = sqlx::query_scalar("SELECT count(*) FROM shipments WHERE order_id = $1")
        .bind(fx.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(shipments, 0);

    // The buyer sees the code to show at the counter.
    let t = tracking(&app, fx.order_id, &fx.buyer).await;
    assert_eq!(t.json["handover_code"], code.as_str(), "{}", t.json);

    let res = deliver(&app, fx.order_id, &code, &staff).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "delivered");
    let (cleared, release_set): (bool, bool) = sqlx::query_as(
        "SELECT handover_code_enc IS NULL, release_at IS NOT NULL FROM orders WHERE id = $1",
    )
    .bind(fx.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(cleared && release_set);
    let collected = audits(&app, fx.order_id, "order.collected").await;
    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].0, Some(staff_id));
    assert!(!collected[0].1.to_string().contains(&code));
    // The release clock started; the money is not released yet.
    let payouts: i64 = sqlx::query_scalar("SELECT count(*) FROM payouts WHERE order_id = $1")
        .bind(fx.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(payouts, 0);
    // Repeating the handover is a no-op.
    let res = deliver(&app, fx.order_id, &code, seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(audits(&app, fx.order_id, "order.collected").await.len(), 1);
}

#[sqlx::test]
async fn wrong_code_lockout_applies(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let fx = paid_collect(&app, 5).await;
    let seller = &fx.listing.seller_token;
    let res = step(&app, fx.order_id, "ready", seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let code = code_of(&app, fx.order_id).await;
    for _ in 0..5 {
        let res = deliver(&app, fx.order_id, &wrong(&code), seller).await;
        assert_eq!(res.status, 422, "{}", res.json);
        assert_eq!(res.json["error"]["code"], "handover_code_wrong");
    }
    // Locked: even the right code is refused for an hour.
    let res = deliver(&app, fx.order_id, &code, seller).await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "handover_locked");
    assert_eq!(status_of(&app, fx.order_id).await, "paid");
    let attempts = audits(&app, fx.order_id, "order.handover_attempt").await;
    assert_eq!(attempts.len(), 6);
    for (_, detail) in &attempts {
        assert!(!detail.to_string().contains(&code));
    }
    // After the lock the right code works.
    sqlx::query(
        "UPDATE orders SET handover_locked_until = now() - interval '1 second' WHERE id = $1",
    )
    .bind(fx.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let res = deliver(&app, fx.order_id, &code, seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
}

#[sqlx::test]
async fn expires_after_window_full_refund_no_strike_stock_back(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let fx = paid_collect(&app, 6).await;
    let variant = fx.listing.variant_id;
    assert_eq!(branch_counts(&app, fx.branch, variant).await, (4, 0));

    // Inside the window nothing happens.
    ready_since(&app, &fx, "4 days 23 hours").await;
    run_due_jobs(&app.state).await;
    assert_eq!(status_of(&app, fx.order_id).await, "paid");

    // A collect order never marked ready is not this job's (the ship deadline's).
    let never_ready = paid_collect(&app, 7).await;
    sqlx::query("UPDATE orders SET ship_by = now() + interval '1 day' WHERE id = $1")
        .bind(never_ready.order_id)
        .execute(&app.pool)
        .await
        .unwrap();

    age_ready(&app, fx.order_id, "5 days 1 minute").await;
    run_due_jobs(&app.state).await;
    let status = status_of(&app, fx.order_id).await;
    assert!(
        status == "refund_pending" || status == "refunded",
        "{status}"
    );
    assert_eq!(status_of(&app, never_ready.order_id).await, "paid");
    let (amount, reason): (i64, String) =
        sqlx::query_as("SELECT amount_kobo, reason FROM refunds WHERE order_id = $1")
            .bind(fx.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!((amount, reason.as_str()), (10_000, "collection_expired"));
    let strikes: i64 =
        sqlx::query_scalar("SELECT count(*) FROM seller_strikes WHERE order_id = $1")
            .bind(fx.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(strikes, 0);
    assert_eq!(branch_counts(&app, fx.branch, variant).await, (5, 0));
    assert_eq!(notices(&app, fx.buyer_id, "collection_expired").await, 1);
    assert_eq!(
        notices(&app, fx.listing.seller_id, "collection_expired").await,
        1
    );
    let rows = audits(&app, fx.order_id, "order.collection_expired").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, None);
    // The code is gone with the order.
    let cleared: bool =
        sqlx::query_scalar("SELECT handover_code_enc IS NULL FROM orders WHERE id = $1")
            .bind(fx.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(cleared);

    // Another run does nothing more.
    run_due_jobs(&app.state).await;
    assert_eq!(
        audits(&app, fx.order_id, "order.collection_expired")
            .await
            .len(),
        1
    );
    assert_eq!(branch_counts(&app, fx.branch, variant).await, (5, 0));
}

#[sqlx::test]
async fn reminders_day_1_and_4_once_each(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let fx = paid_collect(&app, 8).await;
    let res = step(&app, fx.order_id, "ready", &fx.listing.seller_token).await;
    assert_eq!(res.status, 200, "{}", res.json);

    run_due_jobs(&app.state).await;
    assert_eq!(notices(&app, fx.buyer_id, "collect_reminder").await, 0);

    age_ready(&app, fx.order_id, "1 day 1 minute").await;
    run_due_jobs(&app.state).await;
    run_due_jobs(&app.state).await;
    assert_eq!(notices(&app, fx.buyer_id, "collect_reminder").await, 1);

    age_ready(&app, fx.order_id, "4 days 1 minute").await;
    run_due_jobs(&app.state).await;
    run_due_jobs(&app.state).await;
    assert_eq!(notices(&app, fx.buyer_id, "collect_reminder").await, 2);
    assert_eq!(status_of(&app, fx.order_id).await, "paid");
    // The team gets no reminders.
    assert_eq!(
        notices(&app, fx.listing.seller_id, "collect_reminder").await,
        0
    );

    // A collected order is reminded no more.
    let other = paid_collect(&app, 9).await;
    ready_since(&app, &other, "1 day 1 minute").await;
    let code = code_of(&app, other.order_id).await;
    let res = deliver(&app, other.order_id, &code, &other.listing.seller_token).await;
    assert_eq!(res.status, 200, "{}", res.json);
    run_due_jobs(&app.state).await;
    assert_eq!(notices(&app, other.buyer_id, "collect_reminder").await, 0);
}

#[sqlx::test]
async fn late_collection_races_expiry_one_wins(pool: PgPool) {
    let app = TestApp::new(pool).await;
    for n in 0..4 {
        let fx = paid_collect(&app, 20 + n).await;
        ready_since(&app, &fx, "5 days 1 minute").await;
        let code = code_of(&app, fx.order_id).await;
        let (expired, handed) = tokio::join!(
            collect::expire_one(&app.state, fx.order_id),
            deliver(&app, fx.order_id, &code, &fx.listing.seller_token),
        );
        let expired = expired.unwrap();
        let status = status_of(&app, fx.order_id).await;
        let refunds: i64 = sqlx::query_scalar("SELECT count(*) FROM refunds WHERE order_id = $1")
            .bind(fx.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
        if expired {
            assert_eq!(handed.status, 409, "{}", handed.json);
            assert!(
                status == "refund_pending" || status == "refunded",
                "{status}"
            );
            assert_eq!(refunds, 1);
            assert!(
                audits(&app, fx.order_id, "order.collected")
                    .await
                    .is_empty()
            );
            assert_eq!(
                branch_counts(&app, fx.branch, fx.listing.variant_id).await,
                (5, 0)
            );
        } else {
            assert_eq!(handed.status, 200, "{}", handed.json);
            assert_eq!(status, "delivered");
            assert_eq!(refunds, 0);
            assert!(
                audits(&app, fx.order_id, "order.collection_expired")
                    .await
                    .is_empty()
            );
            assert_eq!(
                branch_counts(&app, fx.branch, fx.listing.variant_id).await,
                (4, 0)
            );
        }
    }
}

/// A collect order late to be made ready is late; once ready, the buyer's own wait
/// is never called the seller's delay.
#[sqlx::test]
async fn no_delay_note_once_ready_for_collection(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let fx = paid_collect(&app, 9).await;
    sqlx::query(
        "UPDATE orders SET promised_date = (now() AT TIME ZONE 'Africa/Lagos')::date - 2 \
         WHERE id = $1",
    )
    .bind(fx.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    // Not ready by the ready date: the branch is late.
    let t = tracking(&app, fx.order_id, &fx.buyer).await;
    assert!(t.json["delay_note"].is_string(), "{}", t.json);

    ready_since(&app, &fx, "1 day").await;
    for token in [&fx.buyer, &fx.listing.seller_token] {
        let t = tracking(&app, fx.order_id, token).await;
        assert_eq!(t.status, 200, "{}", t.json);
        assert!(t.json.get("delay_note").is_none(), "{}", t.json);
    }
}

fn step_keys(json: &Value) -> Vec<&str> {
    json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["key"].as_str().unwrap())
        .collect()
}

fn current_step(json: &Value) -> Option<&str> {
    json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["current"] == true)
        .map(|s| s["key"].as_str().unwrap())
}

/// A collect order's timeline has no courier steps: paid, ready, collected.
#[sqlx::test]
async fn collect_timeline_has_its_own_steps(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let fx = paid_collect(&app, 10).await;
    let t = tracking(&app, fx.order_id, &fx.buyer).await;
    assert_eq!(t.status, 200, "{}", t.json);
    assert_eq!(
        step_keys(&t.json),
        vec!["paid", "ready", "collected", "confirmed"],
        "{}",
        t.json
    );
    assert_eq!(current_step(&t.json), Some("paid"), "{}", t.json);
    let first = &t.json["steps"][0];
    assert!(first["at"].is_string() && first["label"].is_string());

    let seller = &fx.listing.seller_token;
    assert_eq!(step(&app, fx.order_id, "ready", seller).await.status, 200);
    let t = tracking(&app, fx.order_id, &fx.buyer).await;
    assert_eq!(current_step(&t.json), Some("ready"), "{}", t.json);
    assert!(t.json["steps"][1]["at"].is_string(), "{}", t.json);

    let code = code_of(&app, fx.order_id).await;
    assert_eq!(deliver(&app, fx.order_id, &code, seller).await.status, 200);
    let t = tracking(&app, fx.order_id, &fx.buyer).await;
    assert_eq!(current_step(&t.json), Some("collected"), "{}", t.json);
    assert!(t.json["steps"][2]["at"].is_string(), "{}", t.json);
}
