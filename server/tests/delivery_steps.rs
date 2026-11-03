//! Seller delivery steps (self-delivery: picked up, out for delivery,
//! delivered with the buyer's handover code) and the courier `ready` step.

mod common;

use axum::http::Method;
use common::TestApp;
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    error::AppError,
    escrow::jobs::run_due_jobs,
    orders::service,
    payments::PaymentStatus,
};
use sqlx::PgPool;
use uuid::Uuid;

const METHOD: &str = "mono_directpay";
const SELLER: &str = "08011111111";
const SELLER2: &str = "08011111112";
const BUYER: &str = "08022222221";

struct Fixture {
    order_id: Uuid,
    seller: String,
    buyer: String,
}

/// Pays the checkout answered by `res` through the fake provider; returns the order id.
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

/// One paid order delivered by the seller (`method` = "self") or by courier.
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
        buyer,
    }
}

async fn step(app: &TestApp, id: Uuid, name: &str, token: &str) -> common::TestResponse {
    app.request(
        Method::POST,
        &format!("/orders/{id}/{name}"),
        None,
        Some(token),
    )
    .await
}

async fn deliver(app: &TestApp, id: Uuid, code: &str, token: &str) -> common::TestResponse {
    app.request(
        Method::POST,
        &format!("/orders/{id}/delivered"),
        Some(json!({ "handover_code": code })),
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

/// The buyer's handover code, decrypted from the order.
async fn code_of(app: &TestApp, id: Uuid) -> String {
    let enc: Vec<u8> = sqlx::query_scalar("SELECT handover_code_enc FROM orders WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    app.state.cipher.decrypt(&enc).unwrap()
}

/// A 6-digit code that is not `code`.
fn wrong(code: &str) -> String {
    if code == "000000" {
        "111111".into()
    } else {
        "000000".into()
    }
}

/// Takes a self-delivery order from `paid` to `out_for_delivery`.
async fn out_for_delivery(app: &TestApp, f: &Fixture) -> String {
    let res = step(app, f.order_id, "picked-up", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = step(app, f.order_id, "out-for-delivery", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    code_of(app, f.order_id).await
}

fn error_code(res: &common::TestResponse) -> &Value {
    &res.json["error"]["code"]
}

#[sqlx::test]
async fn self_delivery_steps_in_order(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    // No code before the order leaves with the seller.
    let none: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT handover_code_enc FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(none.is_none());
    // Out of order: not picked up yet.
    let res = step(&app, f.order_id, "out-for-delivery", &f.seller).await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(error_code(&res), "invalid_order_state");

    let res = step(&app, f.order_id, "picked-up", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "picked_up");
    // Idempotent in its target state.
    let res = step(&app, f.order_id, "picked-up", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);

    let res = step(&app, f.order_id, "out-for-delivery", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "out_for_delivery");
    let code = code_of(&app, f.order_id).await;
    assert_eq!(code.len(), 6);
    assert!(code.bytes().all(|b| b.is_ascii_digit()));
    // A repeat keeps the same code.
    let res = step(&app, f.order_id, "out-for-delivery", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(code_of(&app, f.order_id).await, code);
    let at: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT out_for_delivery_at FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(at.is_some());

    let res = deliver(&app, f.order_id, &code, &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(status_of(&app, f.order_id).await, "completed");
    let reason: String =
        sqlx::query_scalar("SELECT release_reason FROM payouts WHERE order_id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(reason, "buyer_confirmed");
    // The code is single use: it is gone once delivered.
    let gone: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT handover_code_enc FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(gone.is_none());
    // A retry changes nothing and books no second payout.
    let res = deliver(&app, f.order_id, &code, &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let payouts: i64 = sqlx::query_scalar("SELECT count(*) FROM payouts WHERE order_id = $1")
        .bind(f.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(payouts, 1);
}

#[sqlx::test]
async fn delivered_without_right_code_is_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    // Before out_for_delivery there is nothing to check a code against.
    let res = step(&app, f.order_id, "picked-up", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = deliver(&app, f.order_id, "123456", &f.seller).await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(error_code(&res), "invalid_order_state");

    let res = step(&app, f.order_id, "out-for-delivery", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let code = code_of(&app, f.order_id).await;
    let res = deliver(&app, f.order_id, &wrong(&code), &f.seller).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "handover_code_wrong");
    // Anything that is not 6 digits is refused the same way, but is not an
    // attempt: it is not counted or audited, so a slip cannot lock a seller out.
    for bad in [
        String::new(),
        "12345".into(),
        format!("{code}0"),
        "abcdef".into(),
    ] {
        let res = deliver(&app, f.order_id, &bad, &f.seller).await;
        assert_eq!(res.status, 422, "{}", res.json);
        assert_eq!(error_code(&res), "handover_code_wrong");
    }
    assert_eq!(status_of(&app, f.order_id).await, "out_for_delivery");
    let failures: i32 = sqlx::query_scalar("SELECT handover_failures FROM orders WHERE id = $1")
        .bind(f.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(failures, 1);
    let attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'order.handover_attempt'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(attempts, 1);
    // A missing code is a bad body, not an attempt.
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/delivered", f.order_id),
            Some(json!({})),
            Some(&f.seller),
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "validation_failed");
    let payouts: i64 = sqlx::query_scalar("SELECT count(*) FROM payouts")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(payouts, 0);
}

#[sqlx::test]
async fn handover_code_locks_after_five_wrong(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let code = out_for_delivery(&app, &f).await;
    for _ in 0..5 {
        let res = deliver(&app, f.order_id, &wrong(&code), &f.seller).await;
        assert_eq!(res.status, 422, "{}", res.json);
        assert_eq!(error_code(&res), "handover_code_wrong");
    }
    let (failures, locked): (i32, bool) = sqlx::query_as(
        "SELECT handover_failures, handover_locked_until > now() + interval '59 minutes' \
         FROM orders WHERE id = $1",
    )
    .bind(f.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!((failures, locked), (0, true));

    // Locked: even the right code is refused.
    let res = deliver(&app, f.order_id, &code, &f.seller).await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(error_code(&res), "handover_locked");
    assert_eq!(status_of(&app, f.order_id).await, "out_for_delivery");
    // So is a malformed entry: the lock answers first.
    for bad in ["", "12345"] {
        let res = deliver(&app, f.order_id, bad, &f.seller).await;
        assert_eq!(res.status, 409, "{}", res.json);
        assert_eq!(error_code(&res), "handover_locked");
    }

    // Once the lock has expired, the right code works.
    sqlx::query(
        "UPDATE orders SET handover_locked_until = now() - interval '1 second' WHERE id = $1",
    )
    .bind(f.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let res = deliver(&app, f.order_id, &code, &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(status_of(&app, f.order_id).await, "completed");
}

#[sqlx::test]
async fn handover_attempt_audit_has_no_code(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let code = out_for_delivery(&app, &f).await;
    let bad = wrong(&code);
    let res = deliver(&app, f.order_id, &bad, &f.seller).await;
    assert_eq!(res.status, 422, "{}", res.json);
    let res = deliver(&app, f.order_id, &code, &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);

    let rows: Vec<(Option<Uuid>, Value)> = sqlx::query_as(
        "SELECT actor_id, detail FROM audit_log \
         WHERE action = 'order.handover_attempt' AND subject_id = $1 ORDER BY id",
    )
    .bind(f.order_id)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1["ok"], false);
    assert_eq!(rows[1].1["ok"], true);
    assert!(rows.iter().all(|(actor, _)| actor.is_some()));
    // Neither code appears anywhere in the audit log.
    let all: Vec<String> = sqlx::query_scalar("SELECT detail::text FROM audit_log")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    for detail in all {
        assert!(!detail.contains(&code), "{detail}");
        assert!(!detail.contains(&bad), "{detail}");
    }
}

#[sqlx::test]
async fn only_the_store_owner_can_step(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let other = app
        .make_active_store_with_product(SELLER2, "Other Shop", 10_000, 5)
        .await
        .seller_token;
    let support = app.make_support().await;
    let owner = app.owner_token().await;
    for token in [&f.buyer, &other, &support, &owner] {
        for name in ["picked-up", "out-for-delivery", "ready"] {
            let res = step(&app, f.order_id, name, token).await;
            assert!(
                res.status == 403 || res.status == 404,
                "{name}: {} {}",
                res.status,
                res.json
            );
        }
    }
    assert_eq!(status_of(&app, f.order_id).await, "paid");

    let code = out_for_delivery(&app, &f).await;
    for token in [&f.buyer, &other, &support, &owner] {
        let res = deliver(&app, f.order_id, &code, token).await;
        assert!(
            res.status == 403 || res.status == 404,
            "{} {}",
            res.status,
            res.json
        );
    }
    assert_eq!(status_of(&app, f.order_id).await, "out_for_delivery");
    // Strangers' attempts never count against the seller.
    let failures: i32 = sqlx::query_scalar("SELECT handover_failures FROM orders WHERE id = $1")
        .bind(f.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(failures, 0);
    // Anonymous callers are refused too.
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/picked-up", f.order_id),
            None,
            None,
        )
        .await;
    assert_eq!(res.status, 401, "{}", res.json);
}

#[sqlx::test]
async fn courier_orders_cannot_use_self_steps(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "courier").await;
    for name in ["picked-up", "out-for-delivery"] {
        let res = step(&app, f.order_id, name, &f.seller).await;
        assert_eq!(res.status, 409, "{name}: {}", res.json);
        assert_eq!(error_code(&res), "invalid_order_state");
    }
    let res = deliver(&app, f.order_id, "123456", &f.seller).await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(error_code(&res), "invalid_order_state");
    assert_eq!(status_of(&app, f.order_id).await, "paid");

    // The courier's own transition (Task 10's InTransit) never makes a code.
    sqlx::query("UPDATE orders SET status = 'picked_up', picked_up_at = now() WHERE id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    server::orders::service::mark_out_for_delivery(
        &mut tx,
        &app.state.cipher,
        f.order_id,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let (status, code): (String, Option<Vec<u8>>) =
        sqlx::query_as("SELECT status::text, handover_code_enc FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(status, "out_for_delivery");
    assert!(code.is_none());
}

#[sqlx::test]
async fn ready_marks_courier_order(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "courier").await;
    let res = step(&app, f.order_id, "ready", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "paid");
    let first: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT ready_at FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(first.is_some());
    // Idempotent: the first time stands.
    let res = step(&app, f.order_id, "ready", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let again: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT ready_at FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(again, first);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'order.ready' AND subject_id = $1",
    )
    .bind(f.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);

    // Idempotent only while `paid`: any later status is refused.
    for status in ["picked_up", "refunded"] {
        sqlx::query("UPDATE orders SET status = $2::order_status WHERE id = $1")
            .bind(f.order_id)
            .bind(status)
            .execute(&app.pool)
            .await
            .unwrap();
        let res = step(&app, f.order_id, "ready", &f.seller).await;
        assert_eq!(res.status, 409, "{status}: {}", res.json);
        assert_eq!(error_code(&res), "invalid_order_state");
    }

    // A self-delivery order is never "ready" for a courier.
    let app2_order = {
        let (_, buyer2) = app.sign_up_verified("08022222229").await;
        let listing = app
            .make_active_store_with_product(SELLER2, "Self Shop", 10_000, 5)
            .await;
        assert_eq!(
            app.set_cart(&buyer2, listing.variant_id, 1).await.status,
            200
        );
        let res = app.checkout(&buyer2, METHOD).await;
        (pay(&app, res).await, listing.seller_token)
    };
    let res = step(&app, app2_order.0, "ready", &app2_order.1).await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(error_code(&res), "invalid_order_state");
}

/// Everywhere `picked_up` may be held, refunded or received, so may
/// `out_for_delivery`.
#[sqlx::test]
async fn out_for_delivery_can_be_held_refunded_or_received(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    out_for_delivery(&app, &f).await;
    // The buyer's "Received" releases it.
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/received", f.order_id),
            Some(json!({ "confirm": true })),
            Some(&f.buyer),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(status_of(&app, f.order_id).await, "completed");

    // A second order: the owner holds it, then refunds it.
    let (_, buyer2) = app.sign_up_verified("08022222229").await;
    let variant: Uuid = sqlx::query_scalar("SELECT id FROM product_variants LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(app.set_cart(&buyer2, variant, 1).await.status, 200);
    let res = app.checkout(&buyer2, METHOD).await;
    let g = Fixture {
        order_id: pay(&app, res).await,
        seller: f.seller.clone(),
        buyer: buyer2,
    };
    let code = out_for_delivery(&app, &g).await;
    let owner = app.owner_token().await;
    let refund = format!("/admin/orders/{}/refund", g.order_id);
    let res = app
        .request(
            Method::POST,
            &refund,
            Some(json!({ "note": "never arrived" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(error_code(&res), "order_not_held");
    let res = app
        .request(
            Method::POST,
            &format!("/admin/orders/{}/hold", g.order_id),
            Some(json!({ "reason": "buyer complaint" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    // Held: the handover still marks it delivered, but nothing is released.
    let res = deliver(&app, g.order_id, &code, &g.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(status_of(&app, g.order_id).await, "delivered");
    let payouts: i64 = sqlx::query_scalar("SELECT count(*) FROM payouts WHERE order_id = $1")
        .bind(g.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(payouts, 0);
    let res = app
        .request(
            Method::POST,
            &refund,
            Some(json!({ "note": "never arrived" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(status_of(&app, g.order_id).await, "refund_pending");
}

async fn has_code(app: &TestApp, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT handover_code_enc IS NOT NULL FROM orders WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// A code is only kept while it can be used.
#[sqlx::test]
async fn handover_code_is_cleared_when_the_order_ends_without_it(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    out_for_delivery(&app, &f).await;
    assert!(has_code(&app, f.order_id).await);
    let owner = app.owner_token().await;
    let res = app
        .request(
            Method::POST,
            &format!("/admin/orders/{}/hold", f.order_id),
            Some(json!({ "reason": "buyer complaint" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = app
        .request(
            Method::POST,
            &format!("/admin/orders/{}/refund", f.order_id),
            Some(json!({ "note": "never arrived" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(!has_code(&app, f.order_id).await);

    // The buyer's "Received" before the handover.
    let (_, buyer2) = app.sign_up_verified("08022222229").await;
    let variant: Uuid = sqlx::query_scalar("SELECT id FROM product_variants LIMIT 1")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(app.set_cart(&buyer2, variant, 1).await.status, 200);
    let res = app.checkout(&buyer2, METHOD).await;
    let g = Fixture {
        order_id: pay(&app, res).await,
        seller: f.seller.clone(),
        buyer: buyer2,
    };
    out_for_delivery(&app, &g).await;
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{}/received", g.order_id),
            Some(json!({ "confirm": true })),
            Some(&g.buyer),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(!has_code(&app, g.order_id).await);
}

/// The courier path (Task 10) delivers from either step, starting the release
/// window; it never releases by itself.
#[sqlx::test]
async fn mark_delivered_accepts_out_for_delivery(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "courier").await;
    let now = time::OffsetDateTime::now_utc();
    service::mark_picked_up(&app.pool, f.order_id, now)
        .await
        .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    service::mark_out_for_delivery(&mut tx, &app.state.cipher, f.order_id, now)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    service::mark_delivered(&app.pool, f.order_id, now)
        .await
        .unwrap();
    assert_eq!(status_of(&app, f.order_id).await, "delivered");
    let release_at: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT release_at FROM orders WHERE id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(release_at.is_some());
}

/// Nothing can skip the handover code on a self-delivery order.
#[sqlx::test]
async fn mark_delivered_refuses_a_self_order(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let now = time::OffsetDateTime::now_utc();
    let res = step(&app, f.order_id, "picked-up", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let err = service::mark_delivered(&app.pool, f.order_id, now)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Conflict("invalid_order_state")));
    assert_eq!(status_of(&app, f.order_id).await, "picked_up");

    let res = step(&app, f.order_id, "out-for-delivery", &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let err = service::mark_delivered(&app.pool, f.order_id, now)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Conflict("invalid_order_state")));
    assert_eq!(status_of(&app, f.order_id).await, "out_for_delivery");
    assert!(has_code(&app, f.order_id).await);
}

/// Attempts racing each other are counted one by one: exactly 5 are compared
/// and the rest meet the one lock they started.
#[sqlx::test]
async fn concurrent_wrong_attempts_make_one_lock(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let code = out_for_delivery(&app, &f).await;
    let bad = wrong(&code);
    let attempt = || deliver(&app, f.order_id, &bad, &f.seller);
    let (a, b, c, d, e, g, h, i, j, k) = tokio::join!(
        attempt(),
        attempt(),
        attempt(),
        attempt(),
        attempt(),
        attempt(),
        attempt(),
        attempt(),
        attempt(),
        attempt()
    );
    let all = [a, b, c, d, e, g, h, i, j, k];
    let count = |status: u16, code: &str| {
        all.iter()
            .filter(|r| r.status == status && error_code(r) == code)
            .count()
    };
    assert_eq!(count(422, "handover_code_wrong"), 5);
    assert_eq!(count(409, "handover_locked"), 5);

    let (failures, locked): (i32, bool) = sqlx::query_as(
        "SELECT handover_failures, handover_locked_until > now() FROM orders WHERE id = $1",
    )
    .bind(f.order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!((failures, locked), (0, true));
    assert_eq!(status_of(&app, f.order_id).await, "out_for_delivery");
}

/// A held order keeps its money through the handover; once the hold lifts,
/// auto-release pays it (the backstop for a release that did not happen).
#[sqlx::test]
async fn held_handover_is_released_by_the_job_after_unhold(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let f = paid_order(&app, "self").await;
    let code = out_for_delivery(&app, &f).await;
    let owner = app.owner_token().await;
    let res = app
        .request(
            Method::POST,
            &format!("/admin/orders/{}/hold", f.order_id),
            Some(json!({ "reason": "buyer complaint" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = deliver(&app, f.order_id, &code, &f.seller).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["held"], true);
    assert_eq!(status_of(&app, f.order_id).await, "delivered");

    // Past the deadline but held: the job leaves it.
    sqlx::query("UPDATE orders SET release_at = now() - interval '1 hour' WHERE id = $1")
        .bind(f.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let report = run_due_jobs(&app.state).await;
    assert_eq!(report.released, 0, "{report:?}");
    assert_eq!(status_of(&app, f.order_id).await, "delivered");

    let res = app
        .request(
            Method::POST,
            &format!("/admin/orders/{}/unhold", f.order_id),
            Some(json!({ "note": "complaint resolved" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let report = run_due_jobs(&app.state).await;
    assert_eq!(report.released, 1, "{report:?}");
    assert_eq!(status_of(&app, f.order_id).await, "completed");
    let reason: String =
        sqlx::query_scalar("SELECT release_reason FROM payouts WHERE order_id = $1")
            .bind(f.order_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(reason, "auto_after_delivery");
}
