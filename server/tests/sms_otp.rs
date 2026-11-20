mod common;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use axum::http::Method;
use common::{TEST_PASSWORD, TestApp};
use serde_json::json;
use server::auth::sms::{SmsError, SmsOtpSender, SmsProvider, code_text};
use sqlx::PgPool;
use tracing_test::traced_test;

const PHONE: &str = "08031234567";
const PHONE_E164: &str = "+2348031234567";

/// Records every message and can be told to fail.
#[derive(Default)]
struct FakeSms {
    sent: Mutex<Vec<(String, String)>>,
    fail: AtomicBool,
}

impl FakeSms {
    fn failing(&self, on: bool) {
        self.fail.store(on, Ordering::SeqCst);
    }

    fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    fn last(&self) -> (String, String) {
        self.sent
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a text was sent")
    }

    /// The six digits in the last text.
    fn last_code(&self) -> String {
        self.last()
            .1
            .split_whitespace()
            .find_map(|w| {
                let w = w.trim_end_matches('.');
                (w.len() == 6 && w.chars().all(|c| c.is_ascii_digit())).then(|| w.to_string())
            })
            .expect("a code in the text")
    }
}

#[async_trait]
impl SmsProvider for FakeSms {
    async fn send(&self, to_e164: &str, text: &str) -> Result<(), SmsError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(SmsError::Rejected);
        }
        self.sent
            .lock()
            .unwrap()
            .push((to_e164.to_string(), text.to_string()));
        Ok(())
    }
}

async fn app_with_sms(pool: PgPool) -> (TestApp, Arc<FakeSms>) {
    let sms = Arc::new(FakeSms::default());
    let app = TestApp::with_otp_sender(pool, Arc::new(SmsOtpSender::new(sms.clone()))).await;
    (app, sms)
}

fn assert_err(res: &common::TestResponse, status: u16, code: &str) {
    assert_eq!(res.status, status, "body: {}", res.json);
    assert_eq!(res.json["error"]["code"], code, "body: {}", res.json);
}

#[sqlx::test]
async fn code_text_is_fixed_copy(pool: PgPool) {
    let (app, sms) = app_with_sms(pool).await;
    let res = app.sign_up(PHONE, TEST_PASSWORD).await;
    assert_eq!(res.status, 201, "body: {}", res.json);
    assert_eq!(res.json["resend_after_secs"], 60);

    let (to, text) = sms.last();
    let code = sms.last_code();
    assert_eq!(to, PHONE_E164);
    assert_eq!(text, code_text(&code));
    assert_eq!(
        text.replace(&code, "123456"),
        "Your code is 123456. It expires in 10 minutes. Never share it."
    );
    assert!(!text.contains("Ada"), "no name in the text");
    // The text works: the code in it verifies the account.
    let ok = app.verify(&res.json["verification_id"], &code).await;
    assert_eq!(ok.status, 200, "body: {}", ok.json);
}

#[sqlx::test]
async fn provider_failure_is_503_without_provider_text(pool: PgPool) {
    let (app, sms) = app_with_sms(pool.clone()).await;
    sms.failing(true);
    let res = app.sign_up(PHONE, TEST_PASSWORD).await;
    assert_err(&res, 503, "otp_unavailable");
    let body = res.json.to_string();
    assert!(
        !body.contains("Rejected") && !body.contains("8031234567"),
        "{body}"
    );
    assert!(res.json["verification_id"].is_null());

    // The code that never left is retired and does not use up the person's quota.
    let live_or_counted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM otp_codes WHERE consumed_at IS NULL OR sent_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live_or_counted, 0, "nothing live and nothing counted");

    // So asking again at once works and sends a new code.
    sms.failing(false);
    let again = app.sign_in(PHONE, TEST_PASSWORD).await;
    assert_err(&again, 403, "phone_unverified");
    assert_eq!(sms.count(), 1);
    let ok = app
        .verify(&again.json["verification_id"], &sms.last_code())
        .await;
    assert_eq!(ok.status, 200, "body: {}", ok.json);
}

#[traced_test]
#[sqlx::test]
async fn phone_and_code_never_logged(pool: PgPool) {
    let (app, sms) = app_with_sms(pool).await;
    sms.failing(true);
    assert_eq!(app.sign_up(PHONE, TEST_PASSWORD).await.status, 503);
    sms.failing(false);
    assert_eq!(app.sign_in(PHONE, TEST_PASSWORD).await.status, 403);
    let code = sms.last_code();
    assert!(!logs_contain(&code), "the code must not be logged");
    assert!(!logs_contain("8031234567"), "the phone must not be logged");
    assert!(!logs_contain(TEST_PASSWORD));
}

#[sqlx::test]
async fn throttled_resend_says_so_and_sends_nothing(pool: PgPool) {
    let (app, sms) = app_with_sms(pool).await;
    let first = app.sign_up(PHONE, "victim-password-1").await;
    assert_eq!(first.status, 201);
    let code = sms.last_code();

    // Someone reclaims the unverified phone within the interval: told to wait, nothing sent,
    // no id for an unsent code, and the original credentials and code are untouched.
    let reclaim = app.sign_up(PHONE, "attacker-password-2").await;
    assert_err(&reclaim, 429, "otp_throttled");
    let wait = reclaim.json["error"]["retry_after_secs"].as_u64().unwrap();
    assert!((1..=60).contains(&wait), "retry_after_secs = {wait}");
    assert!(reclaim.json["verification_id"].is_null());
    assert_eq!(sms.count(), 1);
    assert_eq!(
        app.verify(&first.json["verification_id"], &code)
            .await
            .status,
        200,
        "the first sign-up still verifies"
    );
    assert_eq!(app.sign_in(PHONE, "victim-password-1").await.status, 200);
}

#[sqlx::test]
async fn resend_during_throttle_rebinds_the_live_code(pool: PgPool) {
    let (app, sms) = app_with_sms(pool).await;
    let first = app.sign_up(PHONE, TEST_PASSWORD).await;
    let code = sms.last_code();
    let resend = app.sign_in(PHONE, TEST_PASSWORD).await;
    assert_err(&resend, 403, "phone_unverified");
    assert_eq!(sms.count(), 1, "nothing new is sent");
    // The id is for the code already on the phone; the old id is dead.
    assert_eq!(
        app.verify(&first.json["verification_id"], &code)
            .await
            .status,
        422
    );
    assert_eq!(
        app.verify(&resend.json["verification_id"], &code)
            .await
            .status,
        200
    );
}

#[sqlx::test]
async fn resend_at_the_hourly_cap_with_no_live_code_is_throttled(pool: PgPool) {
    let (app, sms) = app_with_sms(pool.clone()).await;
    assert_eq!(app.sign_up(PHONE, TEST_PASSWORD).await.status, 201);
    for _ in 0..4 {
        app.age_otps(120).await;
        assert_err(
            &app.sign_in(PHONE, TEST_PASSWORD).await,
            403,
            "phone_unverified",
        );
    }
    assert_eq!(sms.count(), 5, "five codes this hour");

    // Every code has expired and the hourly cap is reached: there is nothing to re-bind.
    sqlx::query("UPDATE otp_codes SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    app.age_otps(120).await;
    let res = app.sign_in(PHONE, TEST_PASSWORD).await;
    assert_err(&res, 429, "otp_throttled");
    let wait = res.json["error"]["retry_after_secs"].as_u64().unwrap();
    assert!((60..=3600).contains(&wait), "retry_after_secs = {wait}");
    assert!(res.json["verification_id"].is_null());
    assert_eq!(sms.count(), 5);
    let unsent: i64 = sqlx::query_scalar("SELECT count(*) FROM otp_codes WHERE sent_at IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unsent, 0, "no code is stored that was never sent");
}

#[sqlx::test]
async fn reset_goes_out_as_the_same_text(pool: PgPool) {
    let (app, sms) = app_with_sms(pool).await;
    let signup = app.sign_up(PHONE, TEST_PASSWORD).await;
    let verified = app
        .verify(&signup.json["verification_id"], &sms.last_code())
        .await;
    assert_eq!(verified.status, 200);
    let res = app
        .request(
            Method::POST,
            "/auth/password-reset",
            Some(json!({ "phone": PHONE })),
            None,
        )
        .await;
    assert_eq!(res.status, 202);
    assert_eq!(sms.count(), 2);
    assert_eq!(sms.last().1, code_text(&sms.last_code()));
}
