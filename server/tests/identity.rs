//! Part 7e-1b: the seller identity check. The provider is the fake behind the trait.

mod common;

use axum::http::Method;
use common::{TEST_PASSWORD, TestApp, TestResponse};
use serde_json::{Value, json};
use server::identity::{
    FailReason, Outcome, Verified,
    fake::{FakeIdentity, SIGNATURE_HEADER},
};
use sqlx::PgPool;
use time::macros::date;
use uuid::Uuid;

const NIN: &str = "12345678901";
const NIN2: &str = "98765432109";

async fn app(pool: PgPool) -> TestApp {
    // Refusals wait for a provider round trip in production; tests do not.
    sqlx::query("UPDATE platform_settings SET identity_refusal_delay_ms = 0")
        .execute(&pool)
        .await
        .unwrap();
    TestApp::new(pool).await
}

/// A seller who has redeemed an access code but has not passed the check.
async fn seller(app: &TestApp, phone: &str) -> (Uuid, String) {
    let owner = app.owner_token().await;
    let (_, code) = app.create_access_code(&owner, json!({})).await;
    let (id, token) = app.sign_up_verified(phone).await;
    assert_eq!(app.redeem(&token, &code).await.status, 200);
    (id, token)
}

async fn new_store(app: &TestApp, token: &str, name: &str) -> Uuid {
    let res = app
        .request(
            Method::POST,
            "/stores",
            Some(json!({
                "name": name,
                "pickup_address": "12 Unique Street, Wuse 2",
                "district": "everything-else",
                "pickup_state": "FCT",
                "delivery_courier": true,
                "delivery_self": true,
                "pickup_area_council": "amac",
                "business_phone": common::STORE_PHONE,
                "self_delivery_fee_kobo": 0
            })),
            Some(token),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    res.json["id"].as_str().unwrap().parse().unwrap()
}

async fn verify_payout(
    app: &TestApp,
    token: &str,
    store: Uuid,
    resolved_name: &str,
) -> TestResponse {
    sqlx::query("UPDATE platform_settings SET payout_provider = 'fake'")
        .execute(&app.pool)
        .await
        .unwrap();
    app.fake.set_lookup(Ok(resolved_name.into()));
    app.request(
        Method::PUT,
        &format!("/stores/{store}/payout-account"),
        Some(json!({
            "bank_code": "058",
            "account_number": "0123456785",
            "password": TEST_PASSWORD
        })),
        Some(token),
    )
    .await
}

async fn store_status(app: &TestApp, store: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM stores WHERE id = $1")
        .bind(store)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

fn verified(name: &str, dob: time::Date) -> Outcome {
    Outcome::Passed(Verified {
        full_name: name.into(),
        date_of_birth: dob,
    })
}

fn good() -> Outcome {
    verified("Ada Obi", date!(1990 - 01 - 01))
}

async fn start(app: &TestApp, token: &str, nin: &str) -> TestResponse {
    app.request(
        Method::POST,
        "/me/identity/start",
        Some(json!({ "nin": nin })),
        Some(token),
    )
    .await
}

async fn deliver(app: &TestApp, provider_ref: &str, outcome: &Outcome) -> TestResponse {
    let (sig, body) = FakeIdentity::webhook(provider_ref, outcome);
    app.send_raw(
        Method::POST,
        "/webhooks/identity/fake",
        &[
            (SIGNATURE_HEADER, &sig),
            ("content-type", "application/json"),
        ],
        body,
    )
    .await
}

async fn latest_ref(app: &TestApp, user: Uuid) -> String {
    sqlx::query_scalar(
        "SELECT provider_ref FROM identity_checks WHERE user_id = $1 \
         ORDER BY started_at DESC LIMIT 1",
    )
    .bind(user)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// Starts with `nin` and delivers `outcome`; returns the status endpoint's answer.
async fn verify(app: &TestApp, user: Uuid, token: &str, nin: &str, outcome: Outcome) -> Value {
    let res = start(app, token, nin).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let r = latest_ref(app, user).await;
    assert_eq!(deliver(app, &r, &outcome).await.status, 200);
    status(app, token).await
}

async fn status(app: &TestApp, token: &str) -> Value {
    let res = app
        .request(Method::GET, "/me/identity", None, Some(token))
        .await;
    assert_eq!(res.status, 200);
    res.json
}

async fn check_row(app: &TestApp, user: Uuid) -> (String, Option<String>) {
    sqlx::query_as(
        "SELECT status, reason FROM identity_checks WHERE user_id = $1 \
         ORDER BY started_at DESC LIMIT 1",
    )
    .bind(user)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// Nothing readable anywhere holds the NIN: not the check rows, not the audit log.
async fn assert_no_nin(app: &TestApp, nin: &str) {
    let like = format!("%{nin}%");
    let rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM identity_checks c WHERE c::text LIKE $1) \
              + (SELECT count(*) FROM audit_log a WHERE a::text LIKE $1) \
              + (SELECT count(*) FROM banned_nins b WHERE b::text LIKE $1) \
              + (SELECT count(*) FROM notifications n WHERE n::text LIKE $1)",
    )
    .bind(like)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(rows, 0, "the NIN appears in stored data");
}

#[sqlx::test]
async fn nin_must_be_exactly_eleven_digits(pool: PgPool) {
    let app = app(pool).await;
    let (_, token) = seller(&app, "08031110001").await;
    for bad in [
        "",
        "1234567890",
        "123456789012",
        "1234567890a",
        "1234 5678901",
    ] {
        let res = start(&app, &token, bad).await;
        assert_eq!(res.status, 422, "{bad:?}: {}", res.json);
        // The answer never echoes what was typed.
        assert!(!res.json.to_string().contains(bad) || bad.is_empty());
    }
    assert_eq!(app.identity.starts(), 0);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM identity_checks")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "a malformed NIN does not count as an attempt");
}

#[sqlx::test]
async fn only_sellers_and_members_may_start(pool: PgPool) {
    let app = app(pool).await;
    let (_, token) = app.sign_up_verified("08031110002").await;
    let res = start(&app, &token, NIN).await;
    assert_eq!(res.status, 403);
    assert_eq!(res.json["error"]["code"], "not_a_seller");
    assert_eq!(start(&app, "nope", NIN).await.status, 401);
}

#[sqlx::test]
async fn a_passed_check_activates_the_stores_and_stores_no_nin(pool: PgPool) {
    let app = app(pool).await;
    let (user, token) = seller(&app, "08031110003").await;
    let store = new_store(&app, &token, "Ada Shop").await;
    let res = verify_payout(&app, &token, store, "ADA OBI").await;
    assert_eq!(res.status, 200, "{}", res.json);
    // Payout verified, identity not: the store waits.
    assert_eq!(store_status(&app, store).await, "pending");
    assert_eq!(status(&app, &token).await["status"], "unverified");

    let res = start(&app, &token, NIN).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["provider"], "fake");
    assert!(res.json["session_token"].as_str().unwrap().len() > 8);
    assert_eq!(app.identity.last_nin().as_deref(), Some(NIN));
    assert_eq!(status(&app, &token).await["status"], "in_progress");
    // One live attempt at a time.
    assert_eq!(
        start(&app, &token, NIN).await.json["error"]["code"],
        "identity_in_progress"
    );

    let r = latest_ref(&app, user).await;
    let res = deliver(&app, &r, &good()).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json["status"], "received");
    let s = status(&app, &token).await;
    assert_eq!(s["status"], "passed");
    assert_eq!(s["verified_name"], "Ada Obi");
    assert_eq!(store_status(&app, store).await, "active");

    // The fingerprint is a 32-byte keyed hash, and the NIN is nowhere.
    let (len, name_plain): (i32, bool) = sqlx::query_as(
        "SELECT octet_length(nin_idx), position('Ada'::bytea IN full_name_enc) > 0 \
         FROM identity_checks WHERE user_id = $1",
    )
    .bind(user)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(len, 32);
    assert!(!name_plain, "the name is encrypted");
    assert_no_nin(&app, NIN).await;
    // A passed notice was queued.
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM notifications WHERE user_id = $1")
            .bind(user)
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(kinds.contains(&"identity_passed".to_string()));
    // Passed users cannot start again.
    assert_eq!(
        start(&app, &token, NIN).await.json["error"]["code"],
        "identity_already_verified"
    );
}

#[sqlx::test]
async fn webhook_checks_the_signature_and_ignores_replays(pool: PgPool) {
    let app = app(pool).await;
    let (user, token) = seller(&app, "08031110004").await;
    assert_eq!(start(&app, &token, NIN).await.status, 200);
    let r = latest_ref(&app, user).await;

    let (_, body) = FakeIdentity::webhook(&r, &good());
    let bad = app
        .send_raw(
            Method::POST,
            "/webhooks/identity/fake",
            &[(SIGNATURE_HEADER, "00ff")],
            body.clone(),
        )
        .await;
    assert_eq!(bad.status, 401);
    let none = app
        .send_raw(Method::POST, "/webhooks/identity/fake", &[], body.clone())
        .await;
    assert_eq!(none.status, 401);
    assert_eq!(check_row(&app, user).await.0, "started", "nothing written");

    let (sig, body) = FakeIdentity::webhook(&r, &good());
    let other = app
        .send_raw(
            Method::POST,
            "/webhooks/identity/other",
            &[(SIGNATURE_HEADER, &sig)],
            body,
        )
        .await;
    assert_eq!(other.status, 404);

    assert_eq!(deliver(&app, &r, &good()).await.json["status"], "received");
    // The same message again, and a contradicting one, change nothing.
    assert_eq!(deliver(&app, &r, &good()).await.json["status"], "ignored");
    let again = deliver(&app, &r, &Outcome::Failed(FailReason::FaceMismatch)).await;
    assert_eq!(again.json["status"], "ignored");
    assert_eq!(check_row(&app, user).await.0, "passed");
    // An unknown reference is acknowledged and ignored.
    assert_eq!(
        deliver(&app, "nope", &good()).await.json["status"],
        "ignored"
    );
    // An oversize body is refused with the standard envelope.
    let big = app
        .send_raw(
            Method::POST,
            "/webhooks/identity/fake",
            &[(SIGNATURE_HEADER, "00")],
            vec![b'x'; 200_000],
        )
        .await;
    assert_eq!(big.status, 413);
}

#[sqlx::test]
async fn a_provider_refusal_fails_the_check_and_says_nothing_about_why(pool: PgPool) {
    let app = app(pool).await;
    let (user, token) = seller(&app, "08031110005").await;
    for (n, reason) in [FailReason::NotFound, FailReason::FaceMismatch]
        .into_iter()
        .enumerate()
    {
        let s = verify(&app, user, &token, NIN, Outcome::Failed(reason)).await;
        assert_eq!(s["status"], "failed", "attempt {n}");
        assert!(s.get("reason").is_none());
        assert_eq!(
            check_row(&app, user).await.1.as_deref(),
            Some(reason.code())
        );
    }
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM notifications WHERE user_id = $1")
            .bind(user)
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert!(kinds.iter().any(|k| k == "identity_declined"));
    assert_no_nin(&app, NIN).await;
}

#[sqlx::test]
async fn unsure_minors_and_name_mismatches_go_to_review(pool: PgPool) {
    let app = app(pool).await;
    for (i, (outcome, reason)) in [
        (Outcome::Unsure, "provider_unsure"),
        (
            verified(
                "Ada Obi",
                time::OffsetDateTime::now_utc()
                    .date()
                    .replace_year(2015)
                    .unwrap(),
            ),
            "under_18",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (user, token) = seller(&app, &format!("0803111010{i}")).await;
        let s = verify(&app, user, &token, &format!("1000000000{i}"), outcome).await;
        assert_eq!(s["status"], "review");
        assert_eq!(check_row(&app, user).await.1.as_deref(), Some(reason));
        // In review: no new attempt until the owner decides.
        assert_eq!(
            start(&app, &token, &format!("1000000000{i}")).await.json["error"]["code"],
            "identity_in_review"
        );
    }

    // Verified name against a verified payout account name.
    let (user, token) = seller(&app, "08031110120").await;
    let store = new_store(&app, &token, "Mule Shop").await;
    // The payout lookup only verifies a name that fits the sign-up name (Ada), so the
    // mismatch with the identity's name is what review is for.
    let res = verify_payout(&app, &token, store, "ADA OBI").await;
    assert_eq!(res.json["status"], "verified", "{}", res.json);
    let outcome = verified("Chidi Okafor", date!(1990 - 01 - 01));
    let s = verify(&app, user, &token, "20000000000", outcome).await;
    assert_eq!(s["status"], "review");
    assert_eq!(
        check_row(&app, user).await.1.as_deref(),
        Some("name_mismatch")
    );
    assert_eq!(store_status(&app, store).await, "pending");
}

#[sqlx::test]
async fn the_owner_decides_a_review_with_a_reason(pool: PgPool) {
    let app = app(pool).await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let (user, token) = seller(&app, "08031110006").await;
    let store = new_store(&app, &token, "Review Shop").await;
    assert_eq!(
        verify_payout(&app, &token, store, "ADA OBI").await.status,
        200
    );
    assert_eq!(
        verify(&app, user, &token, NIN, Outcome::Unsure).await["status"],
        "review"
    );
    let id: Uuid = sqlx::query_scalar("SELECT id FROM identity_checks WHERE user_id = $1")
        .bind(user)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let path = format!("/admin/identity/{id}/decide");

    // Support may not decide; a decision needs a reason and a known verdict.
    let res = app
        .request(
            Method::POST,
            &path,
            Some(json!({"decision": "pass", "reason": "ok"})),
            Some(&support),
        )
        .await;
    assert_eq!(res.status, 403);
    for body in [
        json!({"decision": "pass"}),
        json!({"decision": "pass", "reason": "  "}),
        json!({"decision": "maybe", "reason": "x"}),
    ] {
        let res = app
            .request(Method::POST, &path, Some(body), Some(&owner))
            .await;
        assert_eq!(res.status, 422, "{}", res.json);
    }

    let queue = app
        .request(
            Method::GET,
            "/admin/identity/review-queue",
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(queue.status, 200);
    assert_eq!(queue.json["items"][0]["status"], "review");

    let res = app
        .request(
            Method::POST,
            &path,
            Some(json!({"decision": "pass", "reason": "Called the seller, matches the shop"})),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(status(&app, &token).await["status"], "passed");
    assert_eq!(store_status(&app, store).await, "active");
    // Decided once.
    let again = app
        .request(
            Method::POST,
            &path,
            Some(json!({"decision": "fail", "reason": "x"})),
            Some(&owner),
        )
        .await;
    assert_eq!(again.json["error"]["code"], "not_in_review");
    // The note is on the row, never in the audit log.
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE detail::text LIKE '%Called the seller%'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0);

    // A failing decision.
    let (user2, token2) = seller(&app, "08031110007").await;
    verify(&app, user2, &token2, NIN2, Outcome::Unsure).await;
    let id2: Uuid = sqlx::query_scalar("SELECT id FROM identity_checks WHERE user_id = $1")
        .bind(user2)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let res = app
        .request(
            Method::POST,
            &format!("/admin/identity/{id2}/decide"),
            Some(json!({"decision": "fail", "reason": "Photo does not match"})),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(status(&app, &token2).await["status"], "failed");
}

#[sqlx::test]
async fn one_nin_one_account_and_refusals_look_alike(pool: PgPool) {
    let app = app(pool).await;
    let (a, a_token) = seller(&app, "08031110008").await;
    assert_eq!(
        verify(&app, a, &a_token, NIN, good()).await["status"],
        "passed"
    );
    let starts = app.identity.starts();

    let (b, b_token) = seller(&app, "08031110009").await;
    let used = start(&app, &b_token, NIN).await;
    assert_eq!(used.status, 409);
    assert_eq!(used.json["error"]["code"], "identity_not_accepted");
    assert_eq!(app.identity.starts(), starts, "no provider call, no cost");

    // A banned NIN gets the very same answer.
    let (c, c_token) = seller(&app, "08031110010").await;
    sqlx::query("INSERT INTO banned_nins (nin_idx, user_id) VALUES (sha256('x'), $1)")
        .bind(c)
        .execute(&app.pool)
        .await
        .unwrap();
    let key = server::identity::fingerprint(
        &common::TEST_NIN_KEY,
        &server::identity::Nin::parse(NIN2).unwrap(),
    );
    sqlx::query("UPDATE banned_nins SET nin_idx = $1")
        .bind(key)
        .execute(&app.pool)
        .await
        .unwrap();
    let banned = start(&app, &c_token, NIN2).await;
    assert_eq!(banned.status, used.status);
    assert_eq!(banned.json, used.json);
    assert_eq!(app.identity.starts(), starts);

    // The refusals count as attempts but keep no fingerprint of the NIN tried.
    let (n, fp): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(nin_idx) FROM identity_checks WHERE user_id = ANY($1)",
    )
    .bind(vec![b, c])
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!((n, fp), (2, 0));
    // The real reason is in the audit log, by code.
    let why: Vec<String> = sqlx::query_scalar(
        "SELECT detail->>'why' FROM audit_log WHERE action = 'identity.refused' ORDER BY id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(why, ["nin_in_use", "banned_nin"]);
    assert_no_nin(&app, NIN2).await;
}

#[sqlx::test]
async fn the_same_nin_cannot_pass_two_accounts_at_once(pool: PgPool) {
    let app = app(pool).await;
    let (a, a_token) = seller(&app, "08031110011").await;
    let (b, b_token) = seller(&app, "08031110012").await;
    // Both start before either passes.
    assert_eq!(start(&app, &a_token, NIN).await.status, 200);
    assert_eq!(start(&app, &b_token, NIN).await.status, 200);
    assert_eq!(
        deliver(&app, &latest_ref(&app, a).await, &good())
            .await
            .status,
        200
    );
    assert_eq!(
        deliver(&app, &latest_ref(&app, b).await, &good())
            .await
            .status,
        200
    );
    assert_eq!(check_row(&app, a).await.0, "passed");
    assert_eq!(
        check_row(&app, b).await,
        ("failed".into(), Some("not_accepted".into()))
    );
}

#[sqlx::test]
async fn attempts_are_capped_per_day_and_in_total(pool: PgPool) {
    let app = app(pool).await;
    let (user, token) = seller(&app, "08031110013").await;
    for i in 0..3 {
        let s = verify(
            &app,
            user,
            &token,
            &format!("3000000000{i}"),
            Outcome::Failed(FailReason::NotFound),
        )
        .await;
        assert_eq!(s["status"], "failed");
    }
    let res = start(&app, &token, NIN).await;
    assert_eq!(res.status, 429);
    assert_eq!(res.json["error"]["code"], "identity_rate_limited");

    // Move those attempts out of the day window; two more allowed, then review.
    sqlx::query("UPDATE identity_checks SET started_at = started_at - interval '2 days'")
        .execute(&app.pool)
        .await
        .unwrap();
    for i in 3..5 {
        let s = verify(
            &app,
            user,
            &token,
            &format!("3000000000{i}"),
            Outcome::Failed(FailReason::NotFound),
        )
        .await;
        assert_eq!(s["status"], "failed");
    }
    sqlx::query("UPDATE identity_checks SET started_at = started_at - interval '2 days'")
        .execute(&app.pool)
        .await
        .unwrap();
    let starts = app.identity.starts();
    let res = start(&app, &token, NIN).await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "identity_in_review");
    assert_eq!(
        app.identity.starts(),
        starts,
        "no provider call after the cap"
    );
    assert_eq!(
        check_row(&app, user).await,
        ("review".into(), Some("attempts_exhausted".into()))
    );
    assert_eq!(status(&app, &token).await["status"], "review");
}

#[sqlx::test]
async fn a_provider_outage_leaves_nothing_stuck_and_costs_no_attempt(pool: PgPool) {
    let app = app(pool).await;
    let (user, token) = seller(&app, "08031110014").await;
    app.identity.set_fail_start(true);
    for _ in 0..4 {
        let res = start(&app, &token, NIN).await;
        assert_eq!(res.status, 503);
        assert_eq!(res.json["error"]["code"], "identity_unavailable");
    }
    assert_eq!(
        check_row(&app, user).await,
        ("failed".into(), Some("provider_error".into()))
    );
    app.identity.set_fail_start(false);
    assert_eq!(
        start(&app, &token, NIN).await.status,
        200,
        "outages did not use the caps"
    );
}

#[sqlx::test]
async fn the_daily_provider_budget_stops_starts_and_alerts(pool: PgPool) {
    let app = app(pool).await;
    sqlx::query("UPDATE platform_settings SET identity_daily_budget = 1")
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, t1) = seller(&app, "08031110015").await;
    let (_, t2) = seller(&app, "08031110016").await;
    assert_eq!(start(&app, &t1, NIN).await.status, 200);
    let res = start(&app, &t2, NIN2).await;
    assert_eq!(res.status, 503);
    assert_eq!(res.json["error"]["code"], "identity_unavailable");
    let alerts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'identity.budget_exhausted'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(alerts, 1);
    // Asking again within the hour does not pile up alerts.
    start(&app, &t2, NIN2).await;
    let alerts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'identity.budget_exhausted'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(alerts, 1);
    // It shows on the owner's alerts list.
    let owner = app.owner_token().await;
    let list = app
        .request(Method::GET, "/admin/alerts", None, Some(&owner))
        .await;
    assert_eq!(list.status, 200, "{}", list.json);
    assert!(
        list.json.to_string().contains("identity.budget_exhausted"),
        "{}",
        list.json
    );
}

#[sqlx::test]
async fn a_ban_blocks_the_nin_and_suspends_stores_and_an_unban_reverses_it(pool: PgPool) {
    let app = app(pool).await;
    let owner = app.owner_token().await;
    let (a, a_token) = seller(&app, "08031110017").await;
    let store = new_store(&app, &a_token, "Scam Shop").await;
    assert_eq!(
        verify_payout(&app, &a_token, store, "ADA OBI").await.status,
        200
    );
    verify(&app, a, &a_token, NIN, good()).await;
    assert_eq!(store_status(&app, store).await, "active");

    let res = app
        .request(
            Method::POST,
            &format!("/admin/moderation/users/{a}/ban"),
            Some(json!({"reason": "fraud"})),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(store_status(&app, store).await, "suspended");

    // A new account cannot verify with the banned NIN.
    let (_, b_token) = seller(&app, "08031110018").await;
    let res = start(&app, &b_token, NIN).await;
    assert_eq!(res.json["error"]["code"], "identity_not_accepted");
    assert_no_nin(&app, NIN).await;

    let res = app
        .request(
            Method::POST,
            &format!("/admin/moderation/users/{a}/unban"),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM banned_nins")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // Back through the normal rules: payout verified and identity passed.
    assert_eq!(store_status(&app, store).await, "active");
}

#[sqlx::test]
async fn a_ban_blocks_nins_that_were_in_flight_but_not_failed_ones(pool: PgPool) {
    let app = app(pool).await;
    let owner = app.owner_token().await;
    let (a, a_token) = seller(&app, "08031110019").await;
    // A failed attempt (a stolen NIN, say) and an in-flight one.
    verify(
        &app,
        a,
        &a_token,
        NIN2,
        Outcome::Failed(FailReason::FaceMismatch),
    )
    .await;
    assert_eq!(start(&app, &a_token, NIN).await.status, 200);
    let res = app
        .request(
            Method::POST,
            &format!("/admin/moderation/users/{a}/ban"),
            Some(json!({"reason": "fraud"})),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM banned_nins WHERE user_id = $1")
        .bind(a)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "only the attempt that did not fail");
    let (_, b_token) = seller(&app, "08031110020").await;
    assert_eq!(start(&app, &b_token, NIN).await.status, 409);
    // The failed attempt's NIN is not blocked: it may belong to the person it was tried on.
    assert_eq!(start(&app, &b_token, NIN2).await.status, 200);
}

#[sqlx::test]
async fn a_revoke_parks_stores_keeps_the_nin_taken_and_needs_the_owner(pool: PgPool) {
    let app = app(pool).await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let (a, a_token) = seller(&app, "08031110021").await;
    let store = new_store(&app, &a_token, "Revoked Shop").await;
    assert_eq!(
        verify_payout(&app, &a_token, store, "ADA OBI").await.status,
        200
    );
    verify(&app, a, &a_token, NIN, good()).await;
    assert_eq!(store_status(&app, store).await, "active");
    let id: Uuid = sqlx::query_scalar("SELECT id FROM identity_checks WHERE user_id = $1")
        .bind(a)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let path = format!("/admin/identity/{id}/revoke");
    assert_eq!(
        app.request(
            Method::POST,
            &path,
            Some(json!({"reason": "fraud found"})),
            Some(&support)
        )
        .await
        .status,
        403
    );
    assert_eq!(
        app.request(Method::POST, &path, Some(json!({})), Some(&owner))
            .await
            .status,
        422
    );
    let res = app
        .request(
            Method::POST,
            &path,
            Some(json!({"reason": "fraud found"})),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(status(&app, &a_token).await["status"], "revoked");
    assert_eq!(store_status(&app, store).await, "pending");
    // Not freed for someone else, and the user cannot simply start again.
    let (_, b_token) = seller(&app, "08031110022").await;
    assert_eq!(
        start(&app, &b_token, NIN).await.json["error"]["code"],
        "identity_not_accepted"
    );
    assert_eq!(
        start(&app, &a_token, NIN2).await.json["error"]["code"],
        "identity_not_accepted"
    );
    // Revoking twice is refused.
    let again = app
        .request(
            Method::POST,
            &path,
            Some(json!({"reason": "again"})),
            Some(&owner),
        )
        .await;
    assert_eq!(again.json["error"]["code"], "not_passed");
}

#[sqlx::test]
async fn changing_the_payout_name_after_a_pass_sends_the_check_back_to_review(pool: PgPool) {
    let app = app(pool).await;
    let (a, token) = seller(&app, "08031110023").await;
    let store = new_store(&app, &token, "Switch Shop").await;
    assert_eq!(
        verify_payout(&app, &token, store, "ADA OBI").await.status,
        200
    );
    verify(&app, a, &token, NIN, good()).await;
    assert_eq!(store_status(&app, store).await, "active");
    let res = verify_payout(&app, &token, store, "ADA OKAFOR").await;
    assert_eq!(res.json["status"], "verified", "{}", res.json);
    assert_eq!(
        check_row(&app, a).await,
        ("review".into(), Some("payout_name_mismatch".into()))
    );
    assert_eq!(store_status(&app, store).await, "pending");
    assert_eq!(status(&app, &token).await["status"], "review");
}

#[sqlx::test]
async fn existing_stores_get_a_grace_period_then_wait(pool: PgPool) {
    let app = app(pool).await;
    let (user, token) = seller(&app, "08031110024").await;
    let store = new_store(&app, &token, "Old Shop").await;
    assert_eq!(
        verify_payout(&app, &token, store, "ADA OBI").await.status,
        200
    );
    assert_eq!(store_status(&app, store).await, "pending");
    // As if it had been active when this shipped.
    sqlx::query("UPDATE stores SET status = 'active', identity_due_at = now() + interval '3 days' WHERE id = $1")
        .bind(store)
        .execute(&app.pool)
        .await
        .unwrap();
    let s = status(&app, &token).await;
    assert_eq!(s["status"], "unverified");
    assert!(s["required_by"].is_string());

    let report = server::escrow::jobs::run_due_jobs(&app.state).await;
    assert_eq!(report.identity_grace, 0);
    assert_eq!(store_status(&app, store).await, "active");
    // One reminder, however often the job runs.
    server::escrow::jobs::run_due_jobs(&app.state).await;
    let reminders: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND kind = 'identity_due'",
    )
    .bind(user)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(reminders, 1);

    sqlx::query("UPDATE stores SET identity_due_at = now() - interval '1 minute' WHERE id = $1")
        .bind(store)
        .execute(&app.pool)
        .await
        .unwrap();
    let report = server::escrow::jobs::run_due_jobs(&app.state).await;
    assert_eq!(report.identity_grace, 1);
    assert_eq!(store_status(&app, store).await, "pending");
    let plot: i64 = sqlx::query_scalar("SELECT count(*) FROM store_plots WHERE store_id = $1")
        .bind(store)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(plot, 1, "the plot is kept");
    // Passing then brings it back.
    assert_eq!(
        verify(&app, user, &token, NIN, good()).await["status"],
        "passed"
    );
    assert_eq!(store_status(&app, store).await, "active");
}

#[sqlx::test]
async fn a_new_store_gets_no_grace(pool: PgPool) {
    let app = app(pool).await;
    let (_, token) = seller(&app, "08031110025").await;
    let store = new_store(&app, &token, "New Shop").await;
    let due: Option<time::OffsetDateTime> =
        sqlx::query_scalar("SELECT identity_due_at FROM stores WHERE id = $1")
            .bind(store)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(due.is_none());
    assert_eq!(
        verify_payout(&app, &token, store, "ADA OBI").await.status,
        200
    );
    assert_eq!(store_status(&app, store).await, "pending");
}

#[sqlx::test]
async fn the_poll_job_finds_results_and_closes_abandoned_attempts(pool: PgPool) {
    let app = app(pool).await;
    let (user, token) = seller(&app, "08031110026").await;
    assert_eq!(start(&app, &token, NIN).await.status, 200);
    let r = latest_ref(&app, user).await;
    // Too young to poll.
    server::escrow::jobs::run_due_jobs(&app.state).await;
    assert_eq!(app.identity.fetches(), 0);

    sqlx::query("UPDATE identity_checks SET started_at = now() - interval '10 minutes'")
        .execute(&app.pool)
        .await
        .unwrap();
    app.identity.set_result(&r, good());
    let report = server::escrow::jobs::run_due_jobs(&app.state).await;
    assert_eq!(report.identity_attempts, 1);
    assert_eq!(status(&app, &token).await["status"], "passed");

    // An attempt that never answers is closed after 30 minutes.
    let (user2, token2) = seller(&app, "08031110027").await;
    assert_eq!(start(&app, &token2, NIN2).await.status, 200);
    sqlx::query(
        "UPDATE identity_checks SET started_at = now() - interval '31 minutes' WHERE user_id = $1",
    )
    .bind(user2)
    .execute(&app.pool)
    .await
    .unwrap();
    server::escrow::jobs::run_due_jobs(&app.state).await;
    assert_eq!(
        check_row(&app, user2).await,
        ("failed".into(), Some("expired".into()))
    );
    // And the seller can try again straight away.
    assert_eq!(start(&app, &token2, NIN2).await.status, 200);
}

#[sqlx::test]
async fn an_abandoned_attempt_is_closed_when_the_seller_starts_again(pool: PgPool) {
    let app = app(pool).await;
    let (user, token) = seller(&app, "08031110028").await;
    assert_eq!(start(&app, &token, NIN).await.status, 200);
    sqlx::query("UPDATE identity_checks SET started_at = now() - interval '31 minutes'")
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(start(&app, &token, NIN).await.status, 200);
    let expired: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_checks WHERE user_id = $1 AND reason = 'expired'",
    )
    .bind(user)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(expired, 1);
}

#[sqlx::test]
async fn support_sees_statuses_only_and_name_reads_are_audited(pool: PgPool) {
    let app = app(pool).await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let (user, token) = seller(&app, "08031110029").await;
    verify(&app, user, &token, NIN, good()).await;
    let path = format!("/admin/users/{user}/identity");

    let res = app.request(Method::GET, &path, None, Some(&support)).await;
    assert_eq!(res.status, 200);
    let row = &res.json["items"][0];
    assert_eq!(row["status"], "passed");
    for hidden in [
        "verified_name",
        "birth_year",
        "provider_ref",
        "decision_note",
    ] {
        assert!(row.get(hidden).is_none(), "{hidden} shown to support");
    }
    assert!(!res.json.to_string().contains("Ada Obi"));
    assert_eq!(
        app.request(Method::GET, &path, None, Some(&token))
            .await
            .status,
        403
    );

    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'admin.identity_read'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(before, 0, "support reads no name");
    let res = app.request(Method::GET, &path, None, Some(&owner)).await;
    assert_eq!(res.json["items"][0]["verified_name"], "Ada Obi");
    assert_eq!(res.json["items"][0]["birth_year"], 1990);
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'admin.identity_read'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(after, 1);
    let leaked: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE detail::text LIKE '%Ada%'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(leaked, 0);
}

#[sqlx::test]
async fn unsuspending_a_store_still_needs_the_identity_check(pool: PgPool) {
    let app = app(pool).await;
    let owner = app.owner_token().await;
    let (_, token) = seller(&app, "08031110030").await;
    let store = new_store(&app, &token, "Held Shop").await;
    assert_eq!(
        verify_payout(&app, &token, store, "ADA OBI").await.status,
        200
    );
    sqlx::query("UPDATE stores SET status = 'suspended' WHERE id = $1")
        .bind(store)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = app
        .request(
            Method::POST,
            &format!("/admin/stores/{store}/unsuspend"),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(store_status(&app, store).await, "pending");
}

#[sqlx::test]
async fn the_check_is_unavailable_without_a_provider_or_key(pool: PgPool) {
    let mut app = app(pool).await;
    let (_, token) = seller(&app, "08031110031").await;
    app.disable_identity(false, true);
    let res = start(&app, &token, NIN).await;
    assert_eq!(res.status, 503);
    assert_eq!(res.json["error"]["code"], "identity_unavailable");
    app.disable_identity(true, false);
    assert_eq!(start(&app, &token, NIN).await.status, 503);
    // A bad NIN is still refused first.
    assert_eq!(start(&app, &token, "12").await.status, 422);
}

#[sqlx::test]
async fn two_deliveries_at_once_apply_once(pool: PgPool) {
    let app = app(pool).await;
    let (user, token) = seller(&app, "08031110032").await;
    let store = new_store(&app, &token, "Race Shop").await;
    assert_eq!(
        verify_payout(&app, &token, store, "ADA OBI").await.status,
        200
    );
    assert_eq!(start(&app, &token, NIN).await.status, 200);
    let r = latest_ref(&app, user).await;
    let (pass, fail) = (good(), Outcome::Failed(FailReason::FaceMismatch));
    let (a, b) = tokio::join!(deliver(&app, &r, &pass), deliver(&app, &r, &fail));
    assert_eq!((a.status, b.status), (200, 200));
    let answers = [a.json["status"].clone(), b.json["status"].clone()];
    assert_eq!(
        answers.iter().filter(|s| *s == "received").count(),
        1,
        "{answers:?}"
    );
    // Whichever won, the store matches the check and nothing is half-applied.
    let (st, _) = check_row(&app, user).await;
    let store_now = store_status(&app, store).await;
    match st.as_str() {
        "passed" => assert_eq!(store_now, "active"),
        "failed" => assert_eq!(store_now, "pending"),
        other => panic!("unexpected {other}"),
    }
    let decided: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action IN ('identity.passed', 'identity.failed')",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(decided, 1);
}

#[sqlx::test]
async fn a_team_member_may_start_their_own_check(pool: PgPool) {
    let app = app(pool).await;
    let shop = app
        .make_active_store_with_product("08031110033", "Team Shop", 10_000, 5)
        .await;
    let (member, token) = app.sign_up_verified("08031110034").await;
    sqlx::query(
        "INSERT INTO store_members (store_id, user_id, role, added_by) \
         VALUES ($1, $2, 'staff', $3)",
    )
    .bind(shop.store_id)
    .bind(member)
    .bind(shop.seller_id)
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(start(&app, &token, NIN).await.status, 200);
    assert_eq!(
        deliver(&app, &latest_ref(&app, member).await, &good())
            .await
            .status,
        200
    );
    assert_eq!(status(&app, &token).await["status"], "passed");
    // Passing as a member does not make them a seller, and moves no store.
    let seller_flag: bool = sqlx::query_scalar("SELECT is_seller FROM users WHERE id = $1")
        .bind(member)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(!seller_flag);
}
