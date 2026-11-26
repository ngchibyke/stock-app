//! Delivery options and checkout by branch group (part 7e-1, task 8): each store's
//! cart lines are routed to its branches, options are quoted per group, and checkout
//! makes one order per group under one payment.

mod common;

use std::time::Duration;

use axum::http::Method;
use common::{Listing, TestApp, TestResponse, group_option};
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    escrow::jobs::run_due_jobs,
    payments::PaymentStatus,
    stock,
};
use sqlx::PgPool;
use uuid::Uuid;

const METHOD: &str = "mono_directpay";
const OWNER: &str = "08071100001";
const BUYER: &str = "08072100001";
/// What the fake courier quotes.
const QUOTE: i64 = 150_000;

async fn shop(app: &TestApp, stock: i32) -> Listing {
    app.make_active_store_with_product(OWNER, "Group Shop", 10_000, stock)
        .await
}

async fn first_branch(app: &TestApp, store: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM store_branches WHERE store_id = $1 AND position = 0")
        .bind(store)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// The owner adds a branch in AMAC with this self-delivery fee.
async fn add_branch(app: &TestApp, l: &Listing, name: &str, fee: i64) -> Uuid {
    let res = app
        .request(
            Method::POST,
            &format!("/stores/{}/branches", l.store_id),
            Some(json!({
                "name": name,
                "pickup_address": format!("{name} Road, Garki"),
                "pickup_area_council": "amac",
                "business_phone": "08035550000",
                "self_delivery_fee_kobo": fee,
            })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    res.json["id"].as_str().unwrap().parse().unwrap()
}

/// Sets on-hand units of `variant` at `branch`.
async fn set_stock(app: &TestApp, l: &Listing, branch: Uuid, variant: Uuid, n: i32) {
    let res = app
        .request(
            Method::PUT,
            &format!("/branches/{branch}/stock"),
            Some(json!({ "items": [{ "variant_id": variant, "stock": n }] })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
}

async fn patch(app: &TestApp, token: &str, path: &str, body: Value) {
    let res = app
        .request(Method::PATCH, path, Some(body), Some(token))
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
}

async fn close(app: &TestApp, l: &Listing, branch: Uuid) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/stores/{}/branches/{branch}/close", l.store_id),
        None,
        Some(&l.seller_token),
    )
    .await
}

/// A second product of the store with no stock anywhere yet; returns its variant.
async fn second_variant(app: &TestApp, l: &Listing) -> Uuid {
    let product = app
        .add_product(&l.seller_token, l.store_id, 5_000, &[(json!({}), 0)])
        .await;
    app.variant_ids(product).await[0]
}

async fn options(app: &TestApp, buyer: &str) -> (Uuid, Value) {
    let address = app.buyer_address(buyer).await;
    let res = app.delivery_options(buyer, address).await;
    assert_eq!(res.status, 200, "{}", res.json);
    (address, res.json)
}

fn groups(json: &Value) -> &Vec<Value> {
    json["stores"][0]["groups"].as_array().unwrap()
}

fn branch_of(group: &Value) -> Uuid {
    group["branch"]["id"].as_str().unwrap().parse().unwrap()
}

/// `(group_id, option_id)` of the group's option of `method`.
fn choice(group: &Value, method: &str) -> (String, Uuid) {
    let option = group["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == method)
        .unwrap_or_else(|| panic!("no {method} option in {group}"));
    (
        group["group_id"].as_str().unwrap().to_string(),
        option["id"].as_str().unwrap().parse().unwrap(),
    )
}

fn code_of(res: &TestResponse) -> &str {
    res.json["error"]["code"].as_str().unwrap_or("")
}

/// `(branch_id, reserved)` per branch holding the variant, in branch order.
async fn reserved_at(app: &TestApp, variant: Uuid) -> Vec<(Uuid, i32)> {
    sqlx::query_as(
        "SELECT bs.branch_id, bs.reserved_quantity FROM branch_stock bs \
         JOIN store_branches b ON b.id = bs.branch_id \
         WHERE bs.variant_id = $1 ORDER BY b.position",
    )
    .bind(variant)
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

async fn order_count(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM orders")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn courier_code(app: &TestApp, branch: Uuid) -> Option<i64> {
    sqlx::query_scalar("SELECT pickup_courier_address_code FROM store_branches WHERE id = $1")
        .bind(branch)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// Pays the checkout in `res` and returns its one order.
async fn pay(app: &TestApp, res: &TestResponse) -> Uuid {
    let reference = res.json["reference"].as_str().unwrap().to_string();
    let total = res.json["total_kobo"].as_i64().unwrap();
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
    res.json["orders"][0]["order_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[sqlx::test]
async fn single_branch_store_one_group_same_totals_as_before(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let first = first_branch(&app, l.store_id).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 2).await.status, 200);
    let (address, json) = options(&app, &buyer).await;

    let store = &json["stores"][0];
    assert_eq!(store["store_id"], json!(l.store_id));
    assert_eq!(store["unavailable"], json!([]));
    let groups = groups(&json);
    assert_eq!(groups.len(), 1, "{json}");
    let g = &groups[0];
    // Name and area council only: no address, no phone before paying.
    assert_eq!(
        g["branch"],
        json!({ "id": first, "name": "Group Shop", "area_council": "amac" })
    );
    assert_eq!(
        g["lines"],
        json!([{ "variant_id": l.variant_id, "quantity": 2 }])
    );
    let group_id = g["group_id"].as_str().unwrap();
    assert_eq!(group_id.len(), 64, "sha256 hex");
    let fees: Vec<(String, i64)> = g["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["method"].as_str().unwrap().to_string(),
                o["fee_kobo"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        fees,
        vec![("self".to_string(), 0), ("courier".to_string(), QUOTE)]
    );
    // The option rows carry the group's branch and fingerprint.
    let rows: Vec<(Uuid, Option<String>)> =
        sqlx::query_as("SELECT branch_id, group_fingerprint FROM delivery_options")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|r| r.0 == first && r.1.as_deref() == Some(group_id))
    );

    let res = app
        .checkout_with(&buyer, METHOD, address, &[choice(g, "courier")])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["total_kobo"], 20_000 + QUOTE);
    let orders = res.json["orders"].as_array().unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0]["items_total_kobo"], 20_000);
    assert_eq!(orders[0]["delivery_fee_kobo"], QUOTE);
    let branch: Uuid = sqlx::query_scalar("SELECT branch_id FROM orders")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(branch, first);
    assert_eq!(reserved_at(&app, l.variant_id).await, vec![(first, 2)]);
}

#[sqlx::test]
async fn cheapest_of_three_branches_wins(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let store = format!("/stores/{}", l.store_id);
    // Self-delivery only, so each branch has exactly one price: its own fee.
    patch(
        &app,
        &l.seller_token,
        &store,
        json!({ "delivery_courier": false, "self_delivery_fee_kobo": 90_000 }),
    )
    .await;
    let first = first_branch(&app, l.store_id).await;
    let b1 = add_branch(&app, &l, "Garki", 50_000).await;
    let b2 = add_branch(&app, &l, "Wuse", 30_000).await;
    // The fourth in rank is never quoted, however cheap.
    let b3 = add_branch(&app, &l, "Kubwa", 10_000).await;
    for b in [b1, b2, b3] {
        set_stock(&app, &l, b, l.variant_id, 5).await;
    }
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let (address, json) = options(&app, &buyer).await;
    let groups = groups(&json);
    assert_eq!(groups.len(), 1, "{json}");
    assert_eq!(branch_of(&groups[0]), b2);
    assert_eq!(groups[0]["branch"]["name"], "Wuse");
    let fees: Vec<i64> = groups[0]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["fee_kobo"].as_i64().unwrap())
        .collect();
    assert_eq!(fees, vec![30_000]);

    let res = app
        .checkout_with(&buyer, METHOD, address, &[choice(&groups[0], "self")])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["orders"][0]["delivery_fee_kobo"], 30_000);
    let branch: Uuid = sqlx::query_scalar("SELECT branch_id FROM orders")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(branch, b2);
    assert_eq!(
        reserved_at(&app, l.variant_id).await,
        vec![(first, 0), (b1, 0), (b2, 1), (b3, 0)]
    );
}

fn methods_of(group: &Value) -> Vec<&str> {
    group["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["method"].as_str().unwrap())
        .collect()
}

/// Collect (fee 0) does not pick the branch: of two full-fill branches the one
/// with the cheaper delivery wins, and collect is offered only where it is on.
#[sqlx::test]
async fn collect_does_not_win_full_fill_choice(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let store = format!("/stores/{}", l.store_id);
    patch(
        &app,
        &l.seller_token,
        &store,
        json!({ "delivery_courier": false, "self_delivery_fee_kobo": 90_000 }),
    )
    .await;
    let first = first_branch(&app, l.store_id).await;
    let garki = add_branch(&app, &l, "Garki", 50_000).await;
    set_stock(&app, &l, garki, l.variant_id, 5).await;
    // The dearer-delivering branch has a counter.
    sqlx::query("UPDATE store_branches SET collect_enabled = true WHERE id = $1")
        .bind(first)
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let (_, json) = options(&app, &buyer).await;
    let gs = groups(&json);
    assert_eq!(gs.len(), 1, "{json}");
    assert_eq!(branch_of(&gs[0]), garki, "{json}");
    assert_eq!(methods_of(&gs[0]), vec!["self"], "{json}");

    // With the counter at the cheaper branch, collect is offered beside delivery.
    sqlx::query("UPDATE store_branches SET collect_enabled = (id = $1) WHERE store_id = $2")
        .bind(garki)
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, json) = options(&app, &buyer).await;
    let gs = groups(&json);
    assert_eq!(branch_of(&gs[0]), garki, "{json}");
    let mut m = methods_of(&gs[0]);
    m.sort_unstable();
    assert_eq!(m, vec!["collect", "self"], "{json}");
}

#[sqlx::test]
async fn split_makes_one_order_per_branch_one_payment(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let first = first_branch(&app, l.store_id).await;
    let other = second_variant(&app, &l).await;
    let garki = add_branch(&app, &l, "Garki", 20_000).await;
    set_stock(&app, &l, garki, other, 5).await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    assert_eq!(app.set_cart(&buyer, other, 2).await.status, 200);
    let (address, json) = options(&app, &buyer).await;
    let groups = groups(&json);
    assert_eq!(groups.len(), 2, "{json}");
    assert_eq!(json["stores"][0]["unavailable"], json!([]));
    let at_first = groups.iter().find(|g| branch_of(g) == first).unwrap();
    let at_garki = groups.iter().find(|g| branch_of(g) == garki).unwrap();
    assert_eq!(
        at_first["lines"],
        json!([{ "variant_id": l.variant_id, "quantity": 1 }])
    );
    assert_eq!(
        at_garki["lines"],
        json!([{ "variant_id": other, "quantity": 2 }])
    );
    assert_ne!(at_first["group_id"], at_garki["group_id"]);

    // Every group must be chosen.
    let res = app
        .checkout_with(&buyer, METHOD, address, &[choice(at_first, "self")])
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(order_count(&app).await, 0);

    let res = app
        .checkout_with(
            &buyer,
            METHOD,
            address,
            &[choice(at_first, "self"), choice(at_garki, "self")],
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    // One payment: items 10,000 + 2 x 5,000, delivery 0 + 20,000.
    assert_eq!(res.json["total_kobo"], 20_000 + 20_000);
    assert_eq!(res.json["orders"].as_array().unwrap().len(), 2);
    let checkouts: i64 = sqlx::query_scalar("SELECT count(*) FROM checkouts WHERE buyer_id = $1")
        .bind(buyer_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(checkouts, 1);
    let mut orders: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        "SELECT branch_id, items_total_kobo, delivery_fee_kobo FROM orders ORDER BY items_total_kobo",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    orders.sort();
    let mut want = vec![(first, 10_000, 0), (garki, 10_000, 20_000)];
    want.sort();
    assert_eq!(orders, want);
    assert_eq!(reserved_at(&app, l.variant_id).await, vec![(first, 1)]);
    assert_eq!(reserved_at(&app, other).await, vec![(first, 0), (garki, 2)]);
}

#[sqlx::test]
async fn unavailable_lines_listed_and_checkout_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let first = first_branch(&app, l.store_id).await;
    let other = second_variant(&app, &l).await;
    let garki = add_branch(&app, &l, "Garki", 0).await;
    set_stock(&app, &l, first, other, 3).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 2).await.status, 200);
    assert_eq!(app.set_cart(&buyer, other, 1).await.status, 200);
    // One unit at each branch: two together, but a line is never split.
    set_stock(&app, &l, first, l.variant_id, 1).await;
    set_stock(&app, &l, garki, l.variant_id, 1).await;

    let (address, json) = options(&app, &buyer).await;
    assert_eq!(
        json["stores"][0]["unavailable"],
        json!([{ "variant_id": l.variant_id, "wanted": 2, "available": 1 }])
    );
    let groups = groups(&json);
    assert_eq!(groups.len(), 1, "{json}");
    assert_eq!(branch_of(&groups[0]), first);
    assert_eq!(
        groups[0]["lines"],
        json!([{ "variant_id": other, "quantity": 1 }])
    );

    // Today's rule: while a line is unavailable, the whole cart is refused.
    let res = app
        .checkout_with(&buyer, METHOD, address, &[choice(&groups[0], "self")])
        .await;
    assert_eq!((res.status, code_of(&res)), (409, "out_of_stock"));
    assert_eq!(order_count(&app).await, 0);
    assert_eq!(app.reserved(l.variant_id).await, 0);
    assert_eq!(app.reserved(other).await, 0);
}

#[sqlx::test]
async fn allocation_changed_when_stock_moves(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let first = first_branch(&app, l.store_id).await;
    let garki = add_branch(&app, &l, "Garki", 0).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 2).await.status, 200);
    let (address, json) = options(&app, &buyer).await;
    let shown = choice(&groups(&json)[0], "self");
    assert_eq!(branch_of(&groups(&json)[0]), first);

    // The stock moves to Garki after the buyer saw the price.
    set_stock(&app, &l, first, l.variant_id, 0).await;
    set_stock(&app, &l, garki, l.variant_id, 5).await;
    let res = app
        .checkout_with(&buyer, METHOD, address, std::slice::from_ref(&shown))
        .await;
    assert_eq!((res.status, code_of(&res)), (409, "allocation_changed"));
    assert_eq!(order_count(&app).await, 0);
    assert_eq!(
        reserved_at(&app, l.variant_id).await,
        vec![(first, 0), (garki, 0)]
    );

    // Asked again, the options route to Garki and checkout goes through.
    let (address, json) = options(&app, &buyer).await;
    assert_eq!(branch_of(&groups(&json)[0]), garki);
    let res = app
        .checkout_with(
            &buyer,
            METHOD,
            address,
            &[choice(&groups(&json)[0], "self")],
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(
        reserved_at(&app, l.variant_id).await,
        vec![(first, 0), (garki, 2)]
    );
}

#[sqlx::test]
async fn allocation_changed_when_branch_closes(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let first = first_branch(&app, l.store_id).await;
    let garki = add_branch(&app, &l, "Garki", 0).await;
    set_stock(&app, &l, garki, l.variant_id, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let (address, json) = options(&app, &buyer).await;
    assert_eq!(branch_of(&groups(&json)[0]), first);
    let shown = choice(&groups(&json)[0], "self");

    // The first branch may close now that checkout routes by branch.
    let res = close(&app, &l, first).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = app.checkout_with(&buyer, METHOD, address, &[shown]).await;
    assert_eq!((res.status, code_of(&res)), (409, "allocation_changed"));
    assert_eq!(order_count(&app).await, 0);

    let (address, json) = options(&app, &buyer).await;
    assert_eq!(branch_of(&groups(&json)[0]), garki);
    let res = app
        .checkout_with(
            &buyer,
            METHOD,
            address,
            &[choice(&groups(&json)[0], "self")],
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let branch: Uuid = sqlx::query_scalar("SELECT branch_id FROM orders")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(branch, garki);
}

#[sqlx::test]
async fn booking_uses_branch_address_code(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let first = first_branch(&app, l.store_id).await;
    let garki = add_branch(&app, &l, "Garki", 0).await;
    set_stock(&app, &l, first, l.variant_id, 0).await;
    set_stock(&app, &l, garki, l.variant_id, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let (address, json) = options(&app, &buyer).await;
    assert_eq!(branch_of(&groups(&json)[0]), garki);

    // The quote is made from Garki's own address code, cached on Garki only.
    let code = courier_code(&app, garki)
        .await
        .expect("garki's code cached");
    assert_eq!(courier_code(&app, first).await, None);
    assert_eq!(app.courier.quote_senders(), vec![code]);

    let res = app
        .checkout_with(&buyer, METHOD, address, &[group_option(&json, "courier")])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let order = pay(&app, &res).await;
    let (branch, sender): (Uuid, Option<i64>) =
        sqlx::query_as("SELECT branch_id, courier_sender_code FROM orders WHERE id = $1")
            .bind(order)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!((branch, sender), (garki, Some(code)));

    // Moving the first branch is not a move of Garki: the order still books.
    patch(
        &app,
        &l.seller_token,
        &format!("/stores/{}", l.store_id),
        json!({ "pickup_address": "1 First Avenue, Wuse" }),
    )
    .await;
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{order}/ready"),
            None,
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    run_due_jobs(&app.state).await;
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM shipments WHERE order_id = $1")
            .bind(order)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(status, "booked");
    let started: Value = sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = 'shipment.booking_started'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(started["sender_stale"], false);

    // A second order from Garki; Garki then moves, so its quote is never booked.
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let (address, json) = options(&app, &buyer).await;
    let res = app
        .checkout_with(&buyer, METHOD, address, &[group_option(&json, "courier")])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let second = pay(&app, &res).await;
    patch(
        &app,
        &l.seller_token,
        &format!("/stores/{}/branches/{garki}", l.store_id),
        json!({ "pickup_address": "8 Moved Street, Garki" }),
    )
    .await;
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{second}/ready"),
            None,
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    run_due_jobs(&app.state).await;
    let (status, error): (String, Option<String>) =
        sqlx::query_as("SELECT status::text, last_error FROM shipments WHERE order_id = $1")
            .bind(second)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), error.as_deref()),
        ("booking_failed", Some("pickup_moved"))
    );
    assert_eq!(app.courier.book_calls(), 1);
}

#[sqlx::test]
async fn self_delivery_fee_from_branch(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let first = first_branch(&app, l.store_id).await;
    let garki = add_branch(&app, &l, "Garki", 45_000).await;
    set_stock(&app, &l, first, l.variant_id, 0).await;
    set_stock(&app, &l, garki, l.variant_id, 5).await;
    // The store-level fee edits the first branch, not Garki.
    patch(
        &app,
        &l.seller_token,
        &format!("/stores/{}", l.store_id),
        json!({ "self_delivery_fee_kobo": 99_000 }),
    )
    .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let (address, json) = options(&app, &buyer).await;
    let g = &groups(&json)[0];
    assert_eq!(branch_of(g), garki);
    let own: Vec<i64> = g["options"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["method"] == "self")
        .map(|o| o["fee_kobo"].as_i64().unwrap())
        .collect();
    assert_eq!(own, vec![45_000]);
    let res = app
        .checkout_with(&buyer, METHOD, address, &[choice(g, "self")])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["orders"][0]["delivery_fee_kobo"], 45_000);
    assert_eq!(res.json["total_kobo"], 10_000 + 45_000);
}

#[sqlx::test]
async fn quotes_capped_at_three_per_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    for name in ["Garki", "Wuse", "Kubwa", "Maitama"] {
        let b = add_branch(&app, &l, name, 0).await;
        set_stock(&app, &l, b, l.variant_id, 5).await;
    }
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let (_, json) = options(&app, &buyer).await;
    assert_eq!(groups(&json).len(), 1, "{json}");
    assert_eq!(app.courier.quote_calls(), 3);
}

/// A branch close and a checkout reserving there serialise on the variant locks:
/// either the order lands first and the close is refused, or the close lands first
/// and the checkout answers `allocation_changed`. Never an order at a closed branch.
#[sqlx::test]
async fn branch_close_races_checkout(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 50).await;
    let first = first_branch(&app, l.store_id).await;
    let garki = add_branch(&app, &l, "Garki", 0).await;
    set_stock(&app, &l, garki, l.variant_id, 50).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);

    // Deterministic: the close holds its locks while the checkout waits behind them.
    let (address, json) = options(&app, &buyer).await;
    assert_eq!(branch_of(&groups(&json)[0]), first);
    let shown = choice(&groups(&json)[0], "self");
    let mut tx = app.pool.begin().await.unwrap();
    sqlx::query("UPDATE store_branches SET closed_at = now() WHERE id = $1")
        .bind(first)
        .execute(&mut *tx)
        .await
        .unwrap();
    stock::close_branch(&mut tx, first).await.unwrap();
    let chosen = [shown];
    let checkout = app.checkout_with(&buyer, METHOD, address, &chosen);
    let commit = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        tx.commit().await.unwrap();
    };
    let (res, ()) = tokio::join!(checkout, commit);
    assert_eq!((res.status, code_of(&res)), (409, "allocation_changed"));
    assert_eq!(order_count(&app).await, 0);
}

/// The same race through the routes, with no help: whichever takes the variant lock
/// first wins, and the other is refused.
#[sqlx::test]
async fn branch_close_and_checkout_serialise(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 50).await;
    let first = first_branch(&app, l.store_id).await;
    let garki = add_branch(&app, &l, "Garki", 0).await;
    set_stock(&app, &l, garki, l.variant_id, 50).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let (address, json) = options(&app, &buyer).await;
    let g = &groups(&json)[0];
    assert_eq!(branch_of(g), first);
    let chosen = [choice(g, "self")];
    let (res, closed) = tokio::join!(
        app.checkout_with(&buyer, METHOD, address, &chosen),
        close(&app, &l, first)
    );
    match (res.status, closed.status) {
        (201, 409) => assert_eq!(code_of(&closed), "branch_has_open_orders"),
        (409, 200) => assert_eq!(code_of(&res), "allocation_changed"),
        other => panic!("unexpected {other:?}: {} / {}", res.json, closed.json),
    }
    let at_closed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM orders o JOIN store_branches b ON b.id = o.branch_id \
         WHERE b.closed_at IS NOT NULL",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(at_closed, 0);
    assert_eq!(
        reserved_at(&app, l.variant_id)
            .await
            .iter()
            .map(|r| r.1)
            .sum::<i32>(),
        i32::from(res.status == 201)
    );
}

/// Two options for the same branch, quoted in separate calls for different lines,
/// would each pass alone but make two orders at one branch: refused before anything
/// is reserved.
#[sqlx::test]
async fn two_groups_from_one_branch_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let branch = first_branch(&app, l.store_id).await;
    let other = second_variant(&app, &l).await;
    set_stock(&app, &l, branch, other, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;

    // Quote the first line alone, then the second line alone.
    app.set_cart(&buyer, l.variant_id, 1).await;
    let (address, first) = options(&app, &buyer).await;
    let a = choice(&groups(&first)[0], "self");
    app.set_cart(&buyer, l.variant_id, 0).await;
    app.set_cart(&buyer, other, 1).await;
    let (_, second) = options(&app, &buyer).await;
    let b = choice(&groups(&second)[0], "self");
    assert_eq!(branch_of(&groups(&first)[0]), branch);
    assert_eq!(branch_of(&groups(&second)[0]), branch);
    assert_ne!(a.0, b.0);

    // Both lines back in the cart: each option still matches its own line.
    app.set_cart(&buyer, l.variant_id, 1).await;
    let before = (
        reserved_at(&app, l.variant_id).await,
        reserved_at(&app, other).await,
    );
    let res = app.checkout_with(&buyer, METHOD, address, &[a, b]).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(code_of(&res), "delivery_option_invalid");
    assert_eq!(
        (
            reserved_at(&app, l.variant_id).await,
            reserved_at(&app, other).await
        ),
        before
    );
    assert_eq!(before.0, vec![(branch, 0)]);
    assert_eq!(order_count(&app).await, 0);
}
