//! One payout fee per store per checkout (part 7e-1, owner decision): a
//! checkout split across several branches of one store is one order per
//! branch, each released on its own. The seller pays the payout fee once, on
//! whichever of those orders is released first; the platform absorbs the
//! transfer fees of the others. The stamp duty, a statutory levy on each
//! transfer, is still charged on every one of them.

mod common;

use axum::http::Method;
use common::{Listing, TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    disputes::decide::{Decision, decide},
    escrow::{
        ReleaseReason, SettlePayoutOutcome, refund::refund_order, release_by_owner, release_order,
        settle_payout,
    },
    money::{self, FeeSchedule},
    orders::service::mark_picked_up,
    payments::{PaymentStatus, PayoutState},
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

const METHOD: &str = "mono_directpay";
const STAFF: &str = "08073300001";
const COMPLAINT: &str = "The zip was torn and the lining was stained when it arrived";
const REPLY: &str = "It left the shop in perfect condition and was packed in a box";
const NOTE: &str = "Photos confirm the damage";

/// A paid checkout of one store split across two branches, its payment old
/// enough for payouts to go at once. `a` is the small order (first branch,
/// under the stamp-duty threshold), `b` the Garki one (over it).
struct Split {
    a: Uuid,
    b: Uuid,
    seller_token: String,
    buyer_token: String,
}

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

fn self_choice(group: &Value) -> (String, Uuid) {
    let option = group["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["method"] == "self")
        .unwrap_or_else(|| panic!("no self option in {group}"));
    (
        group["group_id"].as_str().unwrap().to_string(),
        option["id"].as_str().unwrap().parse().unwrap(),
    )
}

async fn pay(app: &TestApp, res: &TestResponse) {
    let reference = res.json["reference"].as_str().unwrap().to_string();
    let total = res.json["total_kobo"].as_i64().unwrap();
    app.fake.set_payment(
        &reference,
        PaymentStatus::Successful {
            amount_kobo: total,
            fee_kobo: 300,
            currency: "NGN".into(),
            held: false,
            reference: reference.clone(),
        },
    );
    assert_eq!(
        settle_payment(&app.state, &reference).await.unwrap(),
        SettleOutcome::Paid
    );
}

/// [`paid_split`] with both orders picked up.
async fn split(app: &TestApp) -> Split {
    split_n(app, 1).await
}

/// [`split`] by the `n`th seller and buyer (1..=9), for several in one test.
async fn split_n(app: &TestApp, n: u8) -> Split {
    let s = paid_split_n(app, n).await;
    for id in [s.a, s.b] {
        mark_picked_up(&app.pool, id, OffsetDateTime::now_utc())
            .await
            .unwrap();
    }
    s
}

async fn paid_split(app: &TestApp) -> Split {
    paid_split_n(app, 1).await
}

async fn paid_split_n(app: &TestApp, n: u8) -> Split {
    let l = app
        .make_active_store_with_product(
            &format!("0807130000{n}"),
            &format!("Split Shop {n}"),
            10_000,
            5,
        )
        .await;
    let product = app
        // Two of these put the Garki order over the stamp-duty threshold;
        // the first branch's order (10,000) stays under it.
        .add_product(&l.seller_token, l.store_id, 600_000, &[(json!({}), 0)])
        .await;
    let other = app.variant_ids(product).await[0];
    let garki = add_branch(app, &l, "Garki", 20_000).await;
    let res = app
        .request(
            Method::PUT,
            &format!("/branches/{garki}/stock"),
            Some(json!({ "items": [{ "variant_id": other, "stock": 5 }] })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);

    let (_, buyer) = app.sign_up_verified(&format!("0807230000{n}")).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    assert_eq!(app.set_cart(&buyer, other, 2).await.status, 200);
    let address = app.buyer_address(&buyer).await;
    let res = app.delivery_options(&buyer, address).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let groups = res.json["stores"][0]["groups"].as_array().unwrap().clone();
    assert_eq!(groups.len(), 2, "{}", res.json);
    let choices: Vec<_> = groups.iter().map(self_choice).collect();
    let res = app.checkout_with(&buyer, METHOD, address, &choices).await;
    assert_eq!(res.status, 201, "{}", res.json);
    pay(app, &res).await;
    let checkout_id: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
    sqlx::query("UPDATE checkouts SET paid_at = now() - interval '10 days' WHERE id = $1")
        .bind(checkout_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM orders WHERE checkout_id = $1 ORDER BY items_total_kobo",
    )
    .bind(checkout_id)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(ids.len(), 2);
    Split {
        a: ids[0],
        b: ids[1],
        seller_token: l.seller_token.clone(),
        buyer_token: buyer,
    }
}

/// The fee schedule snapshotted on the order.
async fn schedule_of(app: &TestApp, order: Uuid) -> FeeSchedule {
    let (small_fee, large_fee, threshold, stamp_duty_kobo, stamp_duty_threshold) = sqlx::query_as(
        "SELECT fee_small_kobo, fee_large_kobo, fee_threshold_kobo, \
                    stamp_duty_kobo, stamp_duty_threshold_kobo FROM orders WHERE id = $1",
    )
    .bind(order)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    FeeSchedule {
        small_fee,
        large_fee,
        threshold,
        stamp_duty_kobo,
        stamp_duty_threshold,
    }
}

/// The order's payout before the payout fee, the fee its own snapshot
/// charges on it (what a lone order pays, stamp duty included), and the
/// stamp duty part alone.
async fn before_and_fee(app: &TestApp, order: Uuid) -> (i64, i64, i64) {
    #[allow(clippy::type_complexity)]
    let (before, small, large, threshold, duty, duty_threshold): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT items_total_kobo - platform_cut_kobo - collection_fee_share_kobo \
                + CASE WHEN delivery_method = 'self' THEN delivery_fee_kobo ELSE 0 END, \
                fee_small_kobo, fee_large_kobo, fee_threshold_kobo, \
                stamp_duty_kobo, stamp_duty_threshold_kobo \
         FROM orders WHERE id = $1",
    )
    .bind(order)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let s = FeeSchedule {
        small_fee: small,
        large_fee: large,
        threshold,
        stamp_duty_kobo: duty,
        stamp_duty_threshold: duty_threshold,
    };
    (
        before,
        money::payout_fee_kobo(before, &s),
        money::stamp_duty_kobo(before, &s),
    )
}

/// `(amount_kobo, fee_kobo, reference)` of the order's payout.
async fn payout_of(app: &TestApp, order: Uuid) -> (i64, i64, String) {
    sqlx::query_as("SELECT amount_kobo, fee_kobo, reference FROM payouts WHERE order_id = $1")
        .bind(order)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn balance(app: &TestApp, account: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(sum(amount_kobo), 0)::bigint FROM ledger_entries \
         WHERE account = $1::ledger_account",
    )
    .bind(account)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn scalar(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&app.pool).await.unwrap()
}

/// Every posting balances, per order and over the whole ledger.
async fn assert_ledger_balances(app: &TestApp) {
    assert_eq!(
        scalar(
            app,
            "SELECT count(*) FROM (SELECT order_id FROM ledger_entries \
             GROUP BY order_id HAVING sum(amount_kobo) <> 0) x"
        )
        .await,
        0
    );
    assert_eq!(
        scalar(
            app,
            "SELECT COALESCE(sum(amount_kobo), 0)::bigint FROM ledger_entries"
        )
        .await,
        0
    );
}

async fn total_fees_charged(app: &TestApp) -> i64 {
    scalar(
        app,
        "SELECT COALESCE(sum(fee_kobo), 0)::bigint FROM payouts",
    )
    .await
}

async fn expected_payout(app: &TestApp, s: &Split, order: Uuid) -> Value {
    let res = app
        .request(
            Method::GET,
            &format!("/orders/{order}"),
            None,
            Some(&s.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    res.json["expected_payout_kobo"].clone()
}

async fn release(app: &TestApp, order: Uuid) {
    release_order(&app.state, order, ReleaseReason::BuyerConfirmed)
        .await
        .unwrap();
}

/// Settles the payout of `order` as successful at a real transfer cost of `cost`.
async fn settle(app: &TestApp, order: Uuid, cost: i64) {
    let (_, _, reference) = payout_of(app, order).await;
    app.fake.set_payout_state(
        &reference,
        PayoutState::Successful {
            fee_kobo: cost,
            amount_kobo: None,
        },
    );
    assert_eq!(
        settle_payout(&app.state, &reference).await.unwrap(),
        SettlePayoutOutcome::Settled {
            status: "successful".into()
        }
    );
}

#[sqlx::test]
async fn split_checkout_pays_one_payout_fee(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = split(&app).await;
    let (before_a, fee_a, duty_a) = before_and_fee(&app, s.a).await;
    let (before_b, fee_b, duty_b) = before_and_fee(&app, s.b).await;
    assert!(fee_a > 0 && fee_b > duty_b);
    // b crosses the stamp-duty threshold, a does not.
    assert!(duty_b > 0 && duty_a == 0);
    // Before either is released, each may be the one that pays.
    assert_eq!(expected_payout(&app, &s, s.b).await, before_b - fee_b);

    release(&app, s.a).await;
    assert_eq!(payout_of(&app, s.a).await.0, before_a - fee_a);
    assert_eq!(payout_of(&app, s.a).await.1, fee_a);
    // Its sibling has paid: the other order's estimate carries only the duty.
    assert_eq!(expected_payout(&app, &s, s.b).await, before_b - duty_b);

    release(&app, s.b).await;
    let (amount_b, charged_b, _) = payout_of(&app, s.b).await;
    // No payout fee, but the stamp duty on its own transfer.
    assert_eq!(charged_b, duty_b);
    assert_eq!(amount_b, before_b - duty_b);
    assert_eq!(expected_payout(&app, &s, s.b).await, before_b - duty_b);
    assert_eq!(total_fees_charged(&app).await, fee_a + duty_b);
    assert_eq!(
        balance(&app, "seller_payable").await,
        before_a + before_b - fee_a - duty_b
    );
    assert_eq!(balance(&app, "escrow").await, 0);
    assert_ledger_balances(&app).await;

    // Both transfers cost the platform; it books each real cost as usual.
    let revenue = balance(&app, "platform_revenue").await;
    let provider = balance(&app, "provider_fees").await;
    settle(&app, s.a, 2_000).await;
    settle(&app, s.b, 2_000).await;
    assert_eq!(balance(&app, "platform_revenue").await, revenue - 4_000);
    assert_eq!(balance(&app, "provider_fees").await, provider + 4_000);
    assert_eq!(balance(&app, "seller_payable").await, 0);
    assert_eq!(
        balance(&app, "seller_paid").await,
        before_a + before_b - fee_a - duty_b
    );
    assert_ledger_balances(&app).await;
}

#[sqlx::test]
async fn refunded_sibling_leaves_the_fee_to_the_released_one(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = paid_split(&app).await;
    // The first order is cancelled before it ships: refunded, never released.
    refund_order(&app.state, s.a, true).await.unwrap();
    mark_picked_up(&app.pool, s.b, OffsetDateTime::now_utc())
        .await
        .unwrap();
    let (before_b, fee_b, _) = before_and_fee(&app, s.b).await;
    release(&app, s.b).await;
    let (amount_b, charged_b, _) = payout_of(&app, s.b).await;
    assert_eq!(charged_b, fee_b);
    assert_eq!(amount_b, before_b - fee_b);
    assert_eq!(total_fees_charged(&app).await, fee_b);
    assert_ledger_balances(&app).await;
}

#[sqlx::test]
async fn later_sibling_released_first_pays_the_fee(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = split(&app).await;
    let (before_a, _, duty_a) = before_and_fee(&app, s.a).await;
    let (before_b, fee_b, _) = before_and_fee(&app, s.b).await;
    release(&app, s.b).await;
    release(&app, s.a).await;
    assert_eq!(payout_of(&app, s.b).await.0, before_b - fee_b);
    assert_eq!(payout_of(&app, s.b).await.1, fee_b);
    // a is under the duty threshold: nothing at all.
    assert_eq!(duty_a, 0);
    assert_eq!(payout_of(&app, s.a).await.0, before_a);
    assert_eq!(payout_of(&app, s.a).await.1, 0);
    assert_eq!(total_fees_charged(&app).await, fee_b);
    assert_ledger_balances(&app).await;
}

/// Exactly one of the two orders paid the payout fee; the other paid only
/// its stamp duty.
async fn assert_one_fee(app: &TestApp, s: &Split) {
    let (_, fee_a, duty_a) = before_and_fee(app, s.a).await;
    let (_, fee_b, duty_b) = before_and_fee(app, s.b).await;
    let (_, charged_a, _) = payout_of(app, s.a).await;
    let (_, charged_b, _) = payout_of(app, s.b).await;
    assert!(
        (charged_a, charged_b) == (fee_a, duty_b) || (charged_a, charged_b) == (duty_a, fee_b),
        "{charged_a} {charged_b}"
    );
}

#[sqlx::test]
async fn concurrent_sibling_releases_charge_one_fee(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = split(&app).await;
    let (a, b) = tokio::join!(
        release_order(&app.state, s.a, ReleaseReason::BuyerConfirmed),
        release_order(&app.state, s.b, ReleaseReason::BuyerConfirmed),
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(scalar(&app, "SELECT count(*) FROM payouts").await, 2);
    assert_one_fee(&app, &s).await;
    assert_ledger_balances(&app).await;
}

/// The owner's release locks the order before `release_in` runs; it takes
/// the sibling locks first too, so it cannot deadlock with a buyer's release
/// of the sibling.
#[sqlx::test]
async fn owner_and_buyer_releasing_siblings_at_once_charge_one_fee(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let s = split(&app).await;
    let (actor, _) = app.sign_up_verified(STAFF).await;
    let (a, b) = tokio::join!(
        release_order(&app.state, s.a, ReleaseReason::BuyerConfirmed),
        release_by_owner(&app.state, s.b, actor, "settled by phone"),
    );
    a.unwrap();
    b.unwrap();
    assert_one_fee(&app, &s).await;
    assert_ledger_balances(&app).await;
}

// ------------------------------------------------- dispute decisions

/// The buyer disputes `order` and the seller answers: `awaiting_decision`
/// (the order is held). Returns the dispute id.
async fn answered_dispute(app: &TestApp, s: &Split, order: Uuid) -> Uuid {
    let evidence = app.ready_photo_for(&s.buyer_token, "dispute").await;
    let res = app
        .request(
            Method::POST,
            &format!("/orders/{order}/dispute"),
            Some(json!({ "reason": "damaged", "body": COMPLAINT, "photo_keys": [evidence] })),
            Some(&s.buyer_token),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let id: Uuid = res.json["id"].as_str().unwrap().parse().unwrap();
    let res = app
        .request(
            Method::POST,
            &format!("/disputes/{id}/response"),
            Some(json!({ "body": REPLY, "photo_keys": [] })),
            Some(&s.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    id
}

/// The platform owner's id (created if the fixture has not made it yet).
async fn owner(app: &TestApp) -> Uuid {
    app.owner_token().await;
    sqlx::query_scalar("SELECT id FROM users WHERE role = 'owner'")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn dispute_release_after_sibling_pays_only_stamp_duty(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let actor = owner(&app).await;
    let s = split(&app).await;
    let (before_a, fee_a, _) = before_and_fee(&app, s.a).await;
    let (before_b, _, duty_b) = before_and_fee(&app, s.b).await;
    assert!(duty_b > 0);
    let dispute = answered_dispute(&app, &s, s.b).await;
    release(&app, s.a).await;
    decide(&app.state, actor, dispute, Decision::Release, NOTE)
        .await
        .unwrap();
    assert_eq!(payout_of(&app, s.a).await.0, before_a - fee_a);
    assert_eq!(payout_of(&app, s.a).await.1, fee_a);
    let (amount_b, charged_b, _) = payout_of(&app, s.b).await;
    assert_eq!(charged_b, duty_b);
    assert_eq!(amount_b, before_b - duty_b);
    assert_eq!(total_fees_charged(&app).await, fee_a + duty_b);
    assert_eq!(balance(&app, "escrow").await, 0);
    assert_ledger_balances(&app).await;
}

#[sqlx::test]
async fn dispute_partial_refund_after_sibling_pays_stamp_duty_on_kept_items(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let actor = owner(&app).await;
    let s = split(&app).await;
    let (_, fee_a, _) = before_and_fee(&app, s.a).await;
    let dispute = answered_dispute(&app, &s, s.b).await;
    release(&app, s.a).await;
    // 1,200,000 of items, 100,000 back: the kept part stays over the duty
    // threshold.
    let refund = 100_000;
    decide(
        &app.state,
        actor,
        dispute,
        Decision::PartialRefund(refund),
        NOTE,
    )
    .await
    .unwrap();
    // The order's cut is now the cut on the kept items; its items total is
    // unchanged, so the kept payout before fee is the formula less the refund.
    let (before_full, _, _) = before_and_fee(&app, s.b).await;
    let kept = before_full - refund;
    let duty = money::stamp_duty_kobo(kept, &schedule_of(&app, s.b).await);
    assert!(duty > 0, "{kept}");
    let (amount_b, charged_b, _) = payout_of(&app, s.b).await;
    assert_eq!(charged_b, duty);
    assert_eq!(amount_b, kept - duty);
    assert_eq!(total_fees_charged(&app).await, fee_a + duty);
    assert_eq!(balance(&app, "escrow").await, 0);
    assert_ledger_balances(&app).await;
}

#[sqlx::test]
async fn dispute_release_first_pays_the_fee_and_sibling_pays_none(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let actor = owner(&app).await;
    let s = split(&app).await;
    let (before_a, _, duty_a) = before_and_fee(&app, s.a).await;
    let (before_b, fee_b, _) = before_and_fee(&app, s.b).await;
    let dispute = answered_dispute(&app, &s, s.b).await;
    decide(&app.state, actor, dispute, Decision::Release, NOTE)
        .await
        .unwrap();
    assert_eq!(payout_of(&app, s.b).await.0, before_b - fee_b);
    assert_eq!(payout_of(&app, s.b).await.1, fee_b);
    release(&app, s.a).await;
    assert_eq!(duty_a, 0);
    assert_eq!(payout_of(&app, s.a).await.0, before_a);
    assert_eq!(payout_of(&app, s.a).await.1, 0);
    assert_eq!(total_fees_charged(&app).await, fee_b);
    assert_ledger_balances(&app).await;
}

/// A dispute decision on one order racing a buyer's release of its sibling:
/// both take the sibling locks first, so neither deadlocks and exactly one
/// pays the fee. Three rounds, each its own checkout.
#[sqlx::test]
async fn dispute_decision_racing_sibling_release_charges_one_fee(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let actor = owner(&app).await;
    for n in 1..=3u8 {
        let s = split_n(&app, n).await;
        let dispute = answered_dispute(&app, &s, s.b).await;
        let (released, decided) = tokio::join!(
            release_order(&app.state, s.a, ReleaseReason::BuyerConfirmed),
            decide(&app.state, actor, dispute, Decision::Release, NOTE),
        );
        released.unwrap();
        decided.unwrap();
        assert_one_fee(&app, &s).await;
    }
    assert_eq!(scalar(&app, "SELECT count(*) FROM payouts").await, 6);
    assert_eq!(balance(&app, "escrow").await, 0);
    assert_ledger_balances(&app).await;
}
