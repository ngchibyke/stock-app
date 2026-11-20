mod common;

use axum::http::Method;
use common::{TEST_PASSWORD, TestApp};
use serde_json::{Value, json};
use sqlx::PgPool;

const PHONE: &str = "08031234567";
const NEW_PASSWORD: &str = "a-brand-new-passphrase-9";

fn assert_err(res: &common::TestResponse, status: u16, code: &str) {
    assert_eq!(res.status, status, "body: {}", res.json);
    assert_eq!(res.json["error"]["code"], code, "body: {}", res.json);
}

fn wrong_code(real: &str) -> &'static str {
    if real == "000000" { "111111" } else { "000000" }
}

async fn request_reset(app: &TestApp, phone: &str) -> common::TestResponse {
    app.request(
        Method::POST,
        "/auth/password-reset",
        Some(json!({ "phone": phone })),
        None,
    )
    .await
}

async fn confirm(
    app: &TestApp,
    verification_id: &Value,
    code: &str,
    new_password: &str,
) -> common::TestResponse {
    app.request(
        Method::POST,
        "/auth/password-reset/confirm",
        Some(json!({
            "verification_id": verification_id,
            "code": code,
            "new_password": new_password,
        })),
        None,
    )
    .await
}

fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

#[sqlx::test]
async fn reset_request_looks_the_same_for_unknown_phones(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.sign_up_verified(PHONE).await;
    let sent_before = app.otp.sent_count();

    let known = request_reset(&app, PHONE).await;
    let unknown = request_reset(&app, "08099998888").await;
    assert_eq!(known.status, 202, "body: {}", known.json);
    assert_eq!(unknown.status, 202, "body: {}", unknown.json);
    assert_eq!(keys(&known.json), keys(&unknown.json));
    assert_eq!(
        known.json["resend_after_secs"],
        unknown.json["resend_after_secs"]
    );
    assert_eq!(
        known.json["verification_id"].as_str().unwrap().len(),
        unknown.json["verification_id"].as_str().unwrap().len()
    );
    assert_eq!(
        app.otp.sent_count(),
        sent_before + 1,
        "only the real account gets a code"
    );
    assert!(app.otp.last_code("+2348099998888").is_none());
}

#[sqlx::test]
async fn reset_changes_the_password_and_the_code_works_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.sign_up_verified(PHONE).await;
    let res = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);

    let done = confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD).await;
    assert_eq!(done.status, 204, "body: {}", done.json);
    assert_eq!(app.sign_in(PHONE, NEW_PASSWORD).await.status, 200);
    assert_err(
        &app.sign_in(PHONE, TEST_PASSWORD).await,
        401,
        "invalid_credentials",
    );

    let again = confirm(
        &app,
        &res.json["verification_id"],
        &code,
        "another-new-pass-77",
    )
    .await;
    assert_err(&again, 422, "code_invalid");
    assert_eq!(app.sign_in(PHONE, NEW_PASSWORD).await.status, 200);
}

#[sqlx::test]
async fn reset_code_is_single_use_and_attempt_limited(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.sign_up_verified(PHONE).await;
    let res = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);

    for _ in 0..5 {
        let bad = confirm(
            &app,
            &res.json["verification_id"],
            wrong_code(&code),
            NEW_PASSWORD,
        )
        .await;
        assert_err(&bad, 422, "code_invalid");
    }
    // The real code no longer works after five wrong guesses.
    let locked = confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD).await;
    assert_err(&locked, 422, "code_invalid");
    assert_err(
        &app.sign_in(PHONE, NEW_PASSWORD).await,
        401,
        "invalid_credentials",
    );
}

#[sqlx::test]
async fn unknown_verification_id_and_expired_code_look_the_same(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    app.sign_up_verified(PHONE).await;
    let res = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);

    let unknown = confirm(&app, &json!("not-a-real-id"), &code, NEW_PASSWORD).await;
    assert_err(&unknown, 422, "code_invalid");

    sqlx::query("UPDATE otp_codes SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    let expired = confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD).await;
    assert_err(&expired, 422, "code_invalid");
    assert_eq!(
        unknown.json["error"]["message"],
        expired.json["error"]["message"]
    );
}

#[sqlx::test]
async fn reset_code_does_not_verify_a_phone_and_the_reverse(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    // A reset code is useless on /auth/verify-phone.
    let (_, _) = app.sign_up_verified(PHONE).await;
    let res = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);
    let as_verify = app.verify(&res.json["verification_id"], &code).await;
    assert_eq!(as_verify.status, 422, "body: {}", as_verify.json);

    // A sign-up code is useless on the reset confirm route, and the account stays unverified.
    let other = "08055550000";
    let signup = app.sign_up(other, TEST_PASSWORD).await;
    let signup_code = app.last_code(other);
    let as_reset = confirm(
        &app,
        &signup.json["verification_id"],
        &signup_code,
        NEW_PASSWORD,
    )
    .await;
    assert_err(&as_reset, 422, "code_invalid");
    let verified: bool =
        sqlx::query_scalar("SELECT phone_verified_at IS NOT NULL FROM users WHERE id = $1")
            .bind(
                signup.json["user_id"]
                    .as_str()
                    .unwrap()
                    .parse::<uuid::Uuid>()
                    .unwrap(),
            )
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!verified);
    // ...and its own code still verifies it.
    assert_eq!(
        app.verify(&signup.json["verification_id"], &signup_code)
            .await
            .status,
        200
    );
}

#[sqlx::test]
async fn reset_revokes_every_session(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, token_one) = app.sign_up_verified(PHONE).await;
    let second = app.sign_in(PHONE, TEST_PASSWORD).await;
    let token_two = second.json["token"].as_str().unwrap().to_string();
    for t in [&token_one, &token_two] {
        assert_eq!(
            app.request(Method::GET, "/me", None, Some(t)).await.status,
            200
        );
    }

    let res = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);
    assert_eq!(
        confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD)
            .await
            .status,
        204
    );
    for t in [&token_one, &token_two] {
        assert_eq!(
            app.request(Method::GET, "/me", None, Some(t)).await.status,
            401,
            "an old session must die"
        );
    }
    assert_err(
        &app.sign_in(PHONE, TEST_PASSWORD).await,
        401,
        "invalid_credentials",
    );
    assert_eq!(app.sign_in(PHONE, NEW_PASSWORD).await.status, 200);
}

#[sqlx::test]
async fn reset_clears_the_lockout(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.sign_up_verified(PHONE).await;
    for _ in 0..10 {
        assert_err(
            &app.sign_in(PHONE, "wrong-password-1").await,
            401,
            "invalid_credentials",
        );
    }
    assert_err(
        &app.sign_in(PHONE, TEST_PASSWORD).await,
        429,
        "too_many_attempts",
    );

    let res = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);
    assert_eq!(
        confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD)
            .await
            .status,
        204
    );
    assert_eq!(app.sign_in(PHONE, NEW_PASSWORD).await.status, 200);
}

#[sqlx::test]
async fn deleted_account_cannot_reset(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    app.sign_up_verified(PHONE).await;
    sqlx::query("UPDATE users SET deleted_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    let sent_before = app.otp.sent_count();
    let res = request_reset(&app, PHONE).await;
    assert_eq!(res.status, 202);
    assert_eq!(
        app.otp.sent_count(),
        sent_before,
        "no code for a deleted account"
    );
}

#[sqlx::test]
async fn a_code_issued_before_deletion_cannot_be_confirmed(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    app.sign_up_verified(PHONE).await;
    let res = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);
    sqlx::query("UPDATE users SET deleted_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    assert_err(
        &confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD).await,
        422,
        "code_invalid",
    );
}

#[sqlx::test]
async fn unverified_account_gets_no_code(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.sign_up(PHONE, TEST_PASSWORD).await;
    let sent_before = app.otp.sent_count();
    let res = request_reset(&app, PHONE).await;
    assert_eq!(res.status, 202);
    assert_eq!(app.otp.sent_count(), sent_before);
}

#[sqlx::test]
async fn weak_new_password_is_refused_and_the_code_survives(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.sign_up_verified(PHONE).await;
    let res = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);

    let weak = confirm(&app, &res.json["verification_id"], &code, "short").await;
    assert_err(&weak, 422, "validation_failed");
    assert_eq!(
        confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD)
            .await
            .status,
        204,
        "a refused password must not burn the code"
    );
}

#[sqlx::test]
async fn a_throttled_request_leaves_the_live_code_alone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    app.sign_up_verified(PHONE).await;
    let first = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);
    let sent = app.otp.sent_count();

    // Within the resend interval: well-formed answer, nothing sent, and the first code lives.
    let second = request_reset(&app, PHONE).await;
    assert_eq!(second.status, 202);
    assert_eq!(app.otp.sent_count(), sent);
    assert_eq!(
        confirm(&app, &first.json["verification_id"], &code, NEW_PASSWORD)
            .await
            .status,
        204
    );
}

#[sqlx::test]
async fn reset_is_rate_limited(pool: PgPool) {
    let app = TestApp::new(pool).await;
    // The same for a phone with no account: the limit must not tell them apart.
    for _ in 0..10 {
        assert_eq!(request_reset(&app, "08099998888").await.status, 202);
    }
    assert_err(
        &request_reset(&app, "08099998888").await,
        429,
        "too_many_attempts",
    );
    // Another phone is unaffected.
    assert_eq!(request_reset(&app, "08077776666").await.status, 202);
}

#[sqlx::test]
async fn malformed_phone_is_a_validation_error(pool: PgPool) {
    let app = TestApp::new(pool).await;
    assert_err(&request_reset(&app, "12345").await, 422, "invalid_phone");
}

#[sqlx::test]
async fn reset_is_audited_without_phone_or_code(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let (user_id, _) = app.sign_up_verified(PHONE).await;
    let res = request_reset(&app, PHONE).await;
    let code = app.last_code(PHONE);
    assert_eq!(
        confirm(&app, &res.json["verification_id"], &code, NEW_PASSWORD)
            .await
            .status,
        204
    );

    let (actor, subject, detail): (Option<uuid::Uuid>, Option<uuid::Uuid>, Value) = sqlx::query_as(
        "SELECT actor_id, subject_id, detail FROM audit_log WHERE action = 'auth.password_reset'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(subject, Some(user_id));
    assert!(actor == Some(user_id) || actor.is_none());
    let text = detail.to_string();
    assert!(!text.contains(&code));
    assert!(!text.contains("8031234567"));
    assert!(!text.contains(NEW_PASSWORD));
}
