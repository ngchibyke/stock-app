//! Partial refunds (a dispute decision): part of the items total goes back to
//! the buyer and the rest is released to the seller, in one transaction. Each
//! test commits `partial_refund_in` in its own transaction, then sends the
//! refund and the payout after the commit, as the decision route will.

mod common;

use axum::http::Method;
use common::{ShippedOrder, TestApp};
use serde_json::{Value, json};
use server::{
    error::AppError,
    escrow::{
        partial::{PartialIn, partial_refund_in},
        refund::send_refund,
        release::ReleasedIn,
        send_payout,
    },
    money::{self, FeeSchedule},
    settings,
};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
struct OrderRow {
    status: String,
    held: bool,
    items_total_kobo: i64,
    delivery_fee_kobo: i64,
    self_delivery: bool,
    platform_cut_bps: i32,
    collection_fee_share_kobo: i64,
    collection_fee_courier_kobo: i64,
    fee_small_kobo: i64,
    fee_large_kobo: i64,
    fee_threshold_kobo: i64,
    stamp_duty_kobo: i64,
    stamp_duty_threshold_kobo: i64,
}

impl OrderRow {
    fn schedule(&self) -> FeeSchedule {
        FeeSchedule {
            small_fee: self.fee_small_kobo,
            large_fee: self.fee_large_kobo,
            threshold: self.fee_threshold_kobo,
            stamp_duty_kobo: self.stamp_duty_kobo,
            stamp_duty_threshold: self.stamp_duty_threshold_kobo,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct RefundRow {
    id: Uuid,
    amount_kobo: i64,
    kind: String,
    reason: String,
    status: String,
    reference: String,
}

#[derive(Debug, sqlx::FromRow)]
struct PayoutRow {
    id: Uuid,
    status: String,
    amount_kobo: i64,
    fee_kobo: i64,
    release_reason: Option<String>,
    last_error: Option<String>,
}

/// What the release formula gives for a partial refund of `refund` (written
/// out here independently of the server's own arithmetic).
#[derive(Debug, PartialEq, Eq)]
struct Expected {
    retained: i64,
    cut: i64,
    payout_fee: i64,
    payout: i64,
    shortfall: i64,
    /// The `release` posting, in posting order, zero entries left out.
    release_rows: Vec<(String, i64)>,
}

fn expected(o: &OrderRow, refund: i64) -> Expected {
    let retained = o.items_total_kobo - refund;
    let cut = money::cut_kobo(retained, o.platform_cut_bps);
    let (seller_delivery, platform_delivery) = if o.self_delivery {
        (o.delivery_fee_kobo, 0)
    } else {
        (0, o.delivery_fee_kobo)
    };
    let before_fee = retained - cut - o.collection_fee_share_kobo + seller_delivery;
    let payout_fee = money::payout_fee_kobo(before_fee, &o.schedule());
    let computed = before_fee - payout_fee;
    let payout = computed.max(0);
    let shortfall = payout - computed;
    let rows = [
        ("escrow", -(retained + o.delivery_fee_kobo)),
        (
            "platform_revenue",
            cut + platform_delivery - shortfall - o.collection_fee_courier_kobo,
        ),
        (
            "provider_fees",
            o.collection_fee_share_kobo + o.collection_fee_courier_kobo,
        ),
        ("platform_revenue", payout_fee),
        ("seller_payable", payout),
    ];
    Expected {
        retained,
        cut,
        payout_fee,
        payout,
        shortfall,
        release_rows: rows
            .into_iter()
            .filter(|(_, a)| *a != 0)
            .map(|(k, a)| (k.to_string(), a))
            .collect(),
    }
}

async fn exec(app: &TestApp, sql: &str) {
    sqlx::query(sql).execute(&app.pool).await.unwrap();
}

/// A shipped order (`method`, `status`) held for a dispute.
async fn held_order(app: &TestApp, method: &str, status: &str) -> ShippedOrder {
    let o = app.shipped_order(method, status).await;
    exec(
        app,
        &format!(
            "UPDATE orders SET held_at = now(), hold_reason = 'dispute' WHERE id = '{}'",
            o.order_id
        ),
    )
    .await;
    o
}

/// The self-delivery fee [`held_self_order_with_fee`] sets on the store.
const SELF_FEE: i64 = 70_000;

/// A held, picked-up self-delivery order (one unit at 10,000) whose buyer
/// paid a self-delivery fee of [`SELF_FEE`] (the `shipped_order` self fixture
/// has none).
async fn held_self_order_with_fee(app: &TestApp) -> Uuid {
    use server::{
        checkout::settle_payment, orders::service::mark_picked_up, payments::PaymentStatus,
    };
    let l = app
        .make_active_store_with_product("08013000001", "Self Fee Store", 10_000, 5)
        .await;
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{}", l.store_id),
            Some(json!({ "self_delivery_fee_kobo": SELF_FEE })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let (_, buyer) = app.sign_up_verified("08023000001").await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let address = app.buyer_address(&buyer).await;
    let opts = app.delivery_options(&buyer, address).await;
    assert_eq!(opts.status, 200, "{}", opts.json);
    let option: Uuid = opts.json["stores"][0]["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "self")
        .expect("a self option")["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let res = app
        .checkout_with(&buyer, "mono_directpay", address, &[(l.store_id, option)])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let reference = res.json["reference"].as_str().unwrap().to_string();
    let total = res.json["total_kobo"].as_i64().unwrap();
    assert_eq!(total, 10_000 + SELF_FEE);
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
    settle_payment(&app.state, &reference).await.unwrap();
    let checkout_id: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
    let order_id: Uuid = sqlx::query_scalar("SELECT id FROM orders WHERE checkout_id = $1")
        .bind(checkout_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    mark_picked_up(&app.pool, order_id, time::OffsetDateTime::now_utc())
        .await
        .unwrap();
    exec(
        app,
        &format!(
            "UPDATE orders SET held_at = now(), hold_reason = 'dispute' WHERE id = '{order_id}'"
        ),
    )
    .await;
    order_id
}

async fn order_row(app: &TestApp, order_id: Uuid) -> OrderRow {
    sqlx::query_as(
        "SELECT status::text AS status, held_at IS NOT NULL AS held, items_total_kobo, \
                delivery_fee_kobo, COALESCE(delivery_method::text = 'self', false) AS self_delivery, \
                platform_cut_bps, collection_fee_share_kobo, collection_fee_courier_kobo, \
                fee_small_kobo, fee_large_kobo, fee_threshold_kobo, stamp_duty_kobo, \
                stamp_duty_threshold_kobo \
         FROM orders WHERE id = $1",
    )
    .bind(order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn refund_row(app: &TestApp, order_id: Uuid) -> RefundRow {
    sqlx::query_as(
        "SELECT id, amount_kobo, kind, reason, status::text AS status, reference \
         FROM refunds WHERE order_id = $1",
    )
    .bind(order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn payout_row(app: &TestApp, order_id: Uuid) -> PayoutRow {
    sqlx::query_as(
        "SELECT id, status::text AS status, amount_kobo, fee_kobo, release_reason, last_error \
         FROM payouts WHERE order_id = $1",
    )
    .bind(order_id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&app.pool).await.unwrap()
}

/// `partial_refund_in` in its own transaction, committed on success (rolled
/// back on an error).
async fn partial(app: &TestApp, order_id: Uuid, refund_kobo: i64) -> Result<PartialIn, AppError> {
    let settings = settings::load(&app.pool).await.unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    let done = partial_refund_in(&mut tx, &settings, order_id, refund_kobo).await?;
    tx.commit().await.unwrap();
    Ok(done)
}

/// [`partial`], then (after the commit) the refund and payout sends.
async fn partial_and_send(app: &TestApp, order_id: Uuid, refund_kobo: i64) -> PartialIn {
    let calls = (app.fake.calls("refund"), app.fake.calls("create_payout"));
    let done = partial(app, order_id, refund_kobo).await.unwrap();
    // Nothing reaches a provider inside the transaction.
    assert_eq!(
        (app.fake.calls("refund"), app.fake.calls("create_payout")),
        calls
    );
    send_refund(&app.state, done.refund_id).await.unwrap();
    assert_eq!(app.fake.calls("refund"), calls.0 + 1);
    if let ReleasedIn::Done { payout_id, .. } = &done.released {
        send_payout(&app.state, *payout_id).await.unwrap();
    }
    done
}

async fn ledger_rows(app: &TestApp, order_id: Uuid, kind: &str) -> Vec<(String, i64)> {
    sqlx::query_as(
        "SELECT account::text, amount_kobo FROM ledger_entries \
         WHERE order_id = $1 AND kind = $2 ORDER BY id",
    )
    .bind(order_id)
    .bind(kind)
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

async fn order_balance(app: &TestApp, order_id: Uuid, account: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(sum(amount_kobo), 0)::bigint FROM ledger_entries \
         WHERE order_id = $1 AND account = $2::ledger_account",
    )
    .bind(order_id)
    .bind(account)
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

fn assert_code(err: AppError, status: u16, code: &str) {
    let (s, c) = match &err {
        AppError::Unprocessable(c, _) => (422, *c),
        AppError::Conflict(c) | AppError::ConflictDetail(c, _) => (409, *c),
        other => panic!("unexpected error {other:?}"),
    };
    assert_eq!((s, c), (status, code), "{err:?}");
}

/// Checks one committed partial refund of `refund` against the formula.
async fn check_amounts(app: &TestApp, order_id: Uuid, refund: i64, done: &PartialIn) {
    let o = order_row(app, order_id).await;
    let e = expected(&o, refund);

    let r = refund_row(app, order_id).await;
    assert_eq!(r.id, done.refund_id);
    assert_eq!(r.amount_kobo, refund);
    assert_eq!(r.kind, "partial");
    assert_eq!(r.reason, "dispute_partial");
    assert!(r.reference.starts_with("RF"));

    let p = payout_row(app, order_id).await;
    assert!(
        matches!(&done.released, ReleasedIn::Done { payout_id, .. } if *payout_id == p.id),
        "{:?}",
        done.released
    );
    assert_eq!(p.amount_kobo, e.payout);
    assert_eq!(p.fee_kobo, e.payout_fee);
    assert_eq!(p.release_reason.as_deref(), Some("owner_decision"));

    assert_eq!(
        ledger_rows(app, order_id, "refund").await,
        [
            ("escrow".to_string(), -refund),
            ("refunds".to_string(), refund),
        ]
    );
    assert_eq!(ledger_rows(app, order_id, "release").await, e.release_rows);
    // No collection-fee absorption: the seller still pays it.
    assert!(ledger_rows(app, order_id, "refund_fee").await.is_empty());
    let keys: Vec<(String, String)> = sqlx::query_as(
        "SELECT kind, ref FROM ledger_postings WHERE kind IN ('refund', 'release', 'refund_fee') \
         AND ref IN ($1, $2) ORDER BY kind",
    )
    .bind(done.refund_id.to_string())
    .bind(order_id.to_string())
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        keys,
        [
            ("refund".to_string(), done.refund_id.to_string()),
            ("release".to_string(), order_id.to_string()),
        ]
    );

    assert_eq!(o.status, "completed");
    assert!(!o.held, "hold cleared");
    let audit = audits(app, "order.partial_refund")
        .await
        .into_iter()
        .find(|d| d["refund_id"] == json!(done.refund_id))
        .expect("order.partial_refund audited");
    assert_eq!(
        audit,
        json!({
            "refund_id": done.refund_id,
            "refund_kobo": refund,
            "retained_kobo": e.retained,
            "cut_kobo": e.cut,
            "original_cut_kobo": money::cut_kobo(o.items_total_kobo, o.platform_cut_bps),
        })
    );
    // The order's cut is now the one the release took.
    let cut: i64 = sqlx::query_scalar("SELECT platform_cut_kobo FROM orders WHERE id = $1")
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(cut, e.cut);
}

// --------------------------------------------------------------- amounts

#[sqlx::test]
async fn partial_refund_amounts_match_formula(pool: PgPool) {
    let app = TestApp::new(pool).await;

    // Self-delivery: the seller keeps the delivery fee.
    let s = held_self_order_with_fee(&app).await;
    let o = order_row(&app, s).await;
    assert!(o.self_delivery);
    assert_eq!(o.delivery_fee_kobo, SELF_FEE);
    assert_eq!(
        (o.collection_fee_share_kobo, o.collection_fee_courier_kobo),
        (10, 0)
    );
    let done = partial_and_send(&app, s, 2_500).await;
    check_amounts(&app, s, 2_500, &done).await;
    // 10,000 items, 2,500 back: 7,500 retained, cut 5% = 375; 7,500 - 375 -
    // 10 share + 70,000 delivery = 77,115 -> fee 3,500 -> 73,615.
    let e = expected(&o, 2_500);
    assert_eq!(
        (e.retained, e.cut, e.payout_fee, e.payout, e.shortfall),
        (7_500, 375, 3_500, 73_615, 0)
    );
    assert_eq!(
        e.release_rows,
        [
            ("escrow".to_string(), -77_500),
            ("platform_revenue".to_string(), 375),
            ("provider_fees".to_string(), 10),
            ("platform_revenue".to_string(), 3_500),
            ("seller_payable".to_string(), 73_615),
        ]
    );
    assert_eq!(payout_row(&app, s).await.amount_kobo, 73_615);

    // Courier: the delivery fee stays with the platform, which paid the courier.
    let c = held_order(&app, "courier", "out_for_delivery").await;
    let o = order_row(&app, c.order_id).await;
    assert!(!o.self_delivery);
    assert!(o.delivery_fee_kobo > 0);
    let before = app.fake.calls("create_payout");
    let settings = settings::load(&app.pool).await.unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    let done = partial_refund_in(&mut tx, &settings, c.order_id, 1_000)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(app.fake.calls("create_payout"), before);
    check_amounts(&app, c.order_id, 1_000, &done).await;
    // 9,000 retained, cut 450; 9,000 - 450 - 1 seller share = 8,549 -> fee
    // 3,500 -> 5,049. The 150,000 courier fee stays platform revenue, less the
    // 9 of the collection fee charged on it.
    let e = expected(&o, 1_000);
    assert_eq!(
        (
            o.delivery_fee_kobo,
            o.collection_fee_share_kobo,
            o.collection_fee_courier_kobo
        ),
        (150_000, 1, 9)
    );
    assert_eq!(
        (e.retained, e.cut, e.payout_fee, e.payout, e.shortfall),
        (9_000, 450, 3_500, 5_049, 0)
    );
    assert_eq!(
        e.release_rows,
        [
            ("escrow".to_string(), -159_000),
            ("platform_revenue".to_string(), 150_441),
            ("provider_fees".to_string(), 10),
            ("platform_revenue".to_string(), 3_500),
            ("seller_payable".to_string(), 5_049),
        ]
    );
}

#[sqlx::test]
async fn partial_refund_bounds(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = held_order(&app, "self", "delivered").await;
    let items = order_row(&app, o.order_id).await.items_total_kobo;
    for bad in [i64::MIN, -1, 0, items, items + 1, i64::MAX] {
        let err = partial(&app, o.order_id, bad).await.unwrap_err();
        assert_code(err, 422, "refund_out_of_range");
    }
    // Nothing changed.
    let row = order_row(&app, o.order_id).await;
    assert_eq!((row.status.as_str(), row.held), ("delivered", true));
    assert_eq!(count(&app, "SELECT count(*) FROM refunds").await, 0);
    assert_eq!(count(&app, "SELECT count(*) FROM payouts").await, 0);

    // The smallest and the largest refund both go through.
    let done = partial_and_send(&app, o.order_id, 1).await;
    check_amounts(&app, o.order_id, 1, &done).await;
    let o2 = held_order(&app, "self", "delivered").await;
    let done = partial_and_send(&app, o2.order_id, items - 1).await;
    check_amounts(&app, o2.order_id, items - 1, &done).await;
}

#[sqlx::test]
async fn partial_refund_requires_a_held_shipped_order(pool: PgPool) {
    let app = TestApp::new(pool).await;
    // Not held.
    let o = app.shipped_order("self", "picked_up").await;
    assert_code(
        partial(&app, o.order_id, 100).await.unwrap_err(),
        409,
        "order_not_held",
    );
    // Already partially refunded (completed, with a payout and a refund).
    let o = held_order(&app, "self", "picked_up").await;
    partial(&app, o.order_id, 100).await.unwrap();
    exec(
        &app,
        &format!(
            "UPDATE orders SET held_at = now() WHERE id = '{}'",
            o.order_id
        ),
    )
    .await;
    assert_code(
        partial(&app, o.order_id, 100).await.unwrap_err(),
        409,
        "already_released",
    );
    assert_eq!(count(&app, "SELECT count(*) FROM refunds").await, 1);
    assert_eq!(count(&app, "SELECT count(*) FROM payouts").await, 1);
    // A paid (not shipped) order.
    let o = held_order(&app, "self", "picked_up").await;
    exec(
        &app,
        &format!(
            "UPDATE orders SET status = 'paid' WHERE id = '{}'",
            o.order_id
        ),
    )
    .await;
    assert_code(
        partial(&app, o.order_id, 100).await.unwrap_err(),
        409,
        "invalid_order_state",
    );
    // Unknown order.
    assert!(matches!(
        partial(&app, Uuid::new_v4(), 100).await,
        Err(AppError::NotFound)
    ));
}

#[sqlx::test]
async fn partial_refund_small_remainder_goes_manual(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = held_order(&app, "courier", "delivered").await;
    let items = order_row(&app, o.order_id).await.items_total_kobo;
    let done = partial_and_send(&app, o.order_id, items - 1).await;
    check_amounts(&app, o.order_id, items - 1, &done).await;
    let p = payout_row(&app, o.order_id).await;
    assert_eq!(p.status, "manual");
    assert_eq!(p.amount_kobo, 0);
    assert_eq!(p.last_error.as_deref(), Some("non_positive_payout"));
    assert!(matches!(
        done.released,
        ReleasedIn::Done {
            status: "manual",
            ..
        }
    ));
    assert_eq!(app.fake.calls("create_payout"), 0);
    // The seller never owes the platform.
    assert_eq!(order_balance(&app, o.order_id, "seller_payable").await, 0);
}

#[sqlx::test]
async fn partial_refund_ledger_balances(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.fake.set_refund_supported(true);
    let mut cases = vec![("self with fee", held_self_order_with_fee(&app).await, 4_000)];
    for (method, refund) in [("self", 4_000), ("courier", 4_000), ("courier", 9_999)] {
        cases.push((
            method,
            held_order(&app, method, "delivered").await.order_id,
            refund,
        ));
    }
    for (method, order_id, refund) in cases {
        partial_and_send(&app, order_id, refund).await;
        // The order's entries net to zero across accounts...
        let net: i64 = sqlx::query_scalar(
            "SELECT COALESCE(sum(amount_kobo), 0)::bigint FROM ledger_entries WHERE order_id = $1",
        )
        .bind(order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
        assert_eq!(net, 0, "{method} {refund}");
        // ...and its escrow is emptied: refund + release took all of it.
        assert_eq!(order_balance(&app, order_id, "escrow").await, 0);
        assert_eq!(order_balance(&app, order_id, "refunds").await, refund);
    }
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM (SELECT txn_id FROM ledger_entries \
             GROUP BY txn_id HAVING sum(amount_kobo) <> 0) x"
        )
        .await,
        0
    );
}

#[sqlx::test]
async fn partial_refund_keeps_order_completed(pool: PgPool) {
    let app = TestApp::new(pool).await;

    // The provider accepts the refund: processing, and the order stays completed.
    app.fake.set_refund_supported(true);
    let o = held_order(&app, "self", "delivered").await;
    let done = partial_and_send(&app, o.order_id, 3_000).await;
    assert_eq!(refund_row(&app, o.order_id).await.status, "processing");
    assert_eq!(app.fake.refunds()[0].amount_kobo, 3_000);
    assert_eq!(order_row(&app, o.order_id).await.status, "completed");
    assert!(audits(&app, "order.refunded").await.is_empty());
    let sent = audits(&app, "refund.partial_sent").await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["refund_id"], json!(done.refund_id));

    // Refunded by hand: mark-refunded completes it, the order stays completed.
    app.fake.set_refund_supported(false);
    let o = held_order(&app, "courier", "delivered").await;
    let done = partial_and_send(&app, o.order_id, 3_000).await;
    assert_eq!(refund_row(&app, o.order_id).await.status, "manual");
    let owner = app.owner_token().await;
    let res = app
        .request(
            Method::POST,
            &format!("/admin/refunds/{}/mark-refunded", done.refund_id),
            Some(json!({ "note": "paid back by transfer", "provider_reference": "NIP-1" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "completed");
    assert_eq!(refund_row(&app, o.order_id).await.status, "completed");
    assert_eq!(order_row(&app, o.order_id).await.status, "completed");
}

#[sqlx::test]
async fn seller_sees_partial_payout(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = held_order(&app, "self", "delivered").await;
    // Before any refund there is no `refunded_kobo` at all.
    let before = view(&app, o.order_id, &o.seller_token).await;
    assert!(before.get("refunded_kobo").is_none(), "{before}");
    assert_eq!(before["platform_cut_kobo"], 500);

    partial_and_send(&app, o.order_id, 2_000).await;
    let p = payout_row(&app, o.order_id).await;
    assert!(p.amount_kobo > 0);
    let seller = view(&app, o.order_id, &o.seller_token).await;
    assert_eq!(seller["status"], "completed");
    assert_eq!(seller["expected_payout_kobo"], json!(p.amount_kobo));
    // 8,000 retained at 5%: the cut beside the payout is the one taken.
    assert_eq!(seller["platform_cut_kobo"], 400);
    assert_eq!(seller["refunded_kobo"], 2_000);
    let buyer = view(&app, o.order_id, &o.buyer_token).await;
    assert_eq!(buyer["refunded_kobo"], 2_000);
    assert!(buyer.get("platform_cut_kobo").is_none());
}

#[sqlx::test]
async fn full_refund_shows_refunded_kobo(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = held_order(&app, "self", "delivered").await;
    let owner = app.owner_token().await;
    let res = app
        .request(
            Method::POST,
            &format!("/admin/orders/{}/refund", o.order_id),
            Some(json!({ "note": "decided for the buyer" })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let row = order_row(&app, o.order_id).await;
    let total = row.items_total_kobo + row.delivery_fee_kobo;
    assert_eq!(
        view(&app, o.order_id, &o.buyer_token).await["refunded_kobo"],
        total
    );
    let seller = view(&app, o.order_id, &o.seller_token).await;
    assert_eq!(seller["refunded_kobo"], total);
    assert_eq!(seller["status"], "cancelled");
}

/// `GET /orders/{id}` as `token`'s user (200).
async fn view(app: &TestApp, order_id: Uuid, token: &str) -> Value {
    let res = app
        .request(
            Method::GET,
            &format!("/orders/{order_id}"),
            None,
            Some(token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    res.json
}
