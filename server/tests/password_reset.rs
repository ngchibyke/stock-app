//! Password reset by phone code (plan 7a, task 7).

mod common;

use axum::http::Method;
use common::{TEST_PASSWORD, TestApp};
use serde_json::{Value, json};
use sqlx::PgPool;

const NEW_PASSWORD: &str = "a-brand-new-password-9";

async fn request(app: &TestApp, phone: &str) -> common::TestResponse {
    app.request(
        Method::POST,
        "/auth/password-reset",
        Some(json!({ "phone": phone })),
        None,
    )
    .await
}

async fn confirm(app: &TestApp, id: &Value, code: &str, password: &str) -> common::TestResponse {
    app.request(
        Method::POST,
        "/auth/password-reset/confirm",
        Some(json!({ "verification_id": id, "code": code, "new_password": password })),
        None,
    )
    .await
}

/// A verified account whose sign-up code is old enough that a reset code is not throttled.
async fn account(app: &TestApp, phone: &str) -> (uuid::Uuid, String) {
    let made = app.sign_up_verified(phone).await;
    app.age_otps(120).await;
    made
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

#[sqlx::test]
async fn reset_request_looks_the_same_for_unknown_phones(pool: PgPool) {
    let app = TestApp::new(pool).await;
    account(&app, "08031112222").await;
    let known = request(&app, "08031112222").await;
    let unknown = request(&app, "08039998888").await;
    assert_eq!(known.status, 202, "{}", known.json);
    assert_eq!(unknown.status, 202, "{}", unknown.json);
    assert_eq!(keys(&known.json), keys(&unknown.json));
    assert_eq!(
        known.json["verification_id"].as_str().unwrap().len(),
        unknown.json["verification_id"].as_str().unwrap().len()
    );
    assert_eq!(
        known.json["resend_after_secs"],
        unknown.json["resend_after_secs"]
    );
    assert_eq!(app.otp.sent_count(), 2); // the sign-up code, and one reset code
}

#[sqlx::test]
async fn reset_changes_the_password(pool: PgPool) {
    let app = TestApp::new(pool).await;
    account(&app, "08031112222").await;
    let res = request(&app, "08031112222").await;
    let code = app.last_code("08031112222");
    let done = confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD).await;
    assert_eq!(done.status, 204, "{}", done.json);
    assert_eq!(app.sign_in("08031112222", TEST_PASSWORD).await.status, 401);
    assert_eq!(app.sign_in("08031112222", NEW_PASSWORD).await.status, 200);
}

#[sqlx::test]
async fn reset_code_is_single_use_and_attempt_limited(pool: PgPool) {
    let app = TestApp::new(pool).await;
    account(&app, "08031112222").await;
    let res = request(&app, "08031112222").await;
    let id = res.json["verification_id"].clone();
    let code = app.last_code("08031112222");

    let first = confirm(&app, &id, &code, NEW_PASSWORD).await;
    assert_eq!(first.status, 204);
    let again = confirm(&app, &id, &code, "another-password-77").await;
    assert_eq!(again.status, 422);
    assert_eq!(again.json["error"]["code"], "code_invalid");

    // Five wrong guesses lock the code, even the right one afterwards.
    app.age_otps(120).await;
    let res = request(&app, "08031112222").await;
    let id = res.json["verification_id"].clone();
    let right = app.last_code("08031112222");
    let wrong = if right == "000000" {
        "000001"
    } else {
        "000000"
    };
    for _ in 0..5 {
        let r = confirm(&app, &id, wrong, NEW_PASSWORD).await;
        assert_eq!(r.status, 422);
        assert_eq!(r.json["error"]["code"], "code_invalid");
    }
    let locked = confirm(&app, &id, &right, NEW_PASSWORD).await;
    assert_eq!(locked.status, 422);
    assert_eq!(locked.json["error"]["code"], "code_invalid");
}

#[sqlx::test]
async fn reset_code_does_not_verify_a_phone_and_the_reverse(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    account(&app, "08031112222").await;

    // A reset code is no good for verify-phone.
    let res = request(&app, "08031112222").await;
    let reset_code = app.last_code("08031112222");
    let as_verify = app.verify(&res.json["verification_id"], &reset_code).await;
    assert_eq!(as_verify.status, 422);

    // A sign-up code is no good for a reset. The account is unverified, so it has none.
    let up = app.sign_up("08035556666", TEST_PASSWORD).await;
    let signup_code = app.last_code("08035556666");
    let as_reset = confirm(
        &app,
        &up.json["verification_id"],
        &signup_code,
        NEW_PASSWORD,
    )
    .await;
    assert_eq!(as_reset.status, 422);
    let (verified,): (bool,) =
        sqlx::query_as("SELECT phone_verified_at IS NOT NULL FROM users WHERE id = $1::uuid")
            .bind(up.json["user_id"].as_str().unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!verified);
}

#[sqlx::test]
async fn reset_revokes_every_session(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, first) = account(&app, "08031112222").await;
    let second = app.sign_in("08031112222", TEST_PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    let res = request(&app, "08031112222").await;
    let code = app.last_code("08031112222");
    assert_eq!(
        confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD)
            .await
            .status,
        204
    );
    for token in [first, second] {
        let me = app.request(Method::GET, "/me", None, Some(&token)).await;
        assert_eq!(me.status, 401);
    }
}

#[sqlx::test]
async fn reset_clears_the_lockout(pool: PgPool) {
    let app = TestApp::new(pool).await;
    account(&app, "08031112222").await;
    for _ in 0..10 {
        app.sign_in("08031112222", "wrong-password-1").await;
    }
    assert_eq!(app.sign_in("08031112222", TEST_PASSWORD).await.status, 429);
    app.age_otps(120).await;
    let res = request(&app, "08031112222").await;
    let code = app.last_code("08031112222");
    assert_eq!(
        confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD)
            .await
            .status,
        204
    );
    assert_eq!(app.sign_in("08031112222", NEW_PASSWORD).await.status, 200);
}

#[sqlx::test]
async fn deleted_account_cannot_reset(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (user, _) = account(&app, "08031112222").await;
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    let before = app.otp.sent_count();
    let res = request(&app, "08031112222").await;
    assert_eq!(res.status, 202);
    assert_eq!(app.otp.sent_count(), before);
}

#[sqlx::test]
async fn unverified_account_gets_no_code(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.sign_up("08031112222", TEST_PASSWORD).await;
    let before = app.otp.sent_count();
    let res = request(&app, "08031112222").await;
    assert_eq!(res.status, 202);
    assert_eq!(app.otp.sent_count(), before);
}

#[sqlx::test]
async fn weak_new_password_is_refused_and_the_code_survives(pool: PgPool) {
    let app = TestApp::new(pool).await;
    account(&app, "08031112222").await;
    let res = request(&app, "08031112222").await;
    let id = res.json["verification_id"].clone();
    let code = app.last_code("08031112222");
    let weak = confirm(&app, &id, &code, "short").await;
    assert_eq!(weak.status, 422);
    let ok = confirm(&app, &id, &code, NEW_PASSWORD).await;
    assert_eq!(ok.status, 204, "{}", ok.json);
}

#[sqlx::test]
async fn reset_is_rate_limited(pool: PgPool) {
    let app = TestApp::new(pool).await;
    account(&app, "08031112222").await;
    // Sign-up used one send. Two requests inside the minimum interval send once.
    let before = app.otp.sent_count();
    request(&app, "08031112222").await;
    request(&app, "08031112222").await;
    assert_eq!(app.otp.sent_count(), before + 1);
    // And the hourly cap holds.
    for _ in 0..8 {
        app.age_otps(120).await;
        let r = request(&app, "08031112222").await;
        assert_eq!(r.status, 202);
    }
    assert!(app.otp.sent_count() <= before + 5);
}

#[sqlx::test]
async fn throttled_request_keeps_the_earlier_code_working(pool: PgPool) {
    let app = TestApp::new(pool).await;
    account(&app, "08031112222").await;
    request(&app, "08031112222").await;
    let code = app.last_code("08031112222");
    let again = request(&app, "08031112222").await; // inside the interval: no new send
    let done = confirm(&app, &again.json["verification_id"], &code, NEW_PASSWORD).await;
    assert_eq!(done.status, 204, "{}", done.json);
}

#[sqlx::test]
async fn reset_is_audited_without_phone_or_code(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (user, _) = account(&app, "08031112222").await;
    let res = request(&app, "08031112222").await;
    let code = app.last_code("08031112222");
    confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD).await;
    let (detail,): (Value,) = sqlx::query_as(
        "SELECT detail FROM audit_log WHERE action = 'auth.password_reset' AND subject_id = $1",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    let text = detail.to_string();
    assert!(!text.contains("0803") && !text.contains(&code), "{text}");
}

#[sqlx::test]
async fn a_bad_phone_is_a_validation_error(pool: PgPool) {
    let app = TestApp::new(pool).await;
    assert_eq!(request(&app, "123").await.status, 422);
}
