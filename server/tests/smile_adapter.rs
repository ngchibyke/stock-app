//! The Smile ID adapter against a mock server: the token request, the webhook signature
//! and result mapping, and that nothing personal reaches an error or a debug string.

use axum::http::{HeaderMap, HeaderValue};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use server::identity::{
    FailReason, IdentityError, IdentityProvider, Nin, Outcome,
    smile::{self, SmileConfig, SmileIdentity},
};
use server::payments::mono::Secret;
use sha2::Sha256;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

const KEY: &str = "smile-api-key-xyz";
const PARTNER: &str = "1234";

fn provider(base: &str) -> SmileIdentity {
    SmileIdentity::new(SmileConfig {
        partner_id: PARTNER.into(),
        api_key: Secret::new(KEY),
        base_url: base.into(),
        callback_url: "https://api.example.ng/webhooks/identity/smile_id".into(),
        privacy_policy_url: "https://example.ng/privacy".into(),
        sandbox: true,
        connect_timeout: smile::CONNECT_TIMEOUT,
        timeout: smile::REQUEST_TIMEOUT,
        insecure_http_for_tests: true,
    })
    .unwrap()
}

fn nin() -> Nin {
    Nin::parse("12345678901").unwrap()
}

fn signed(timestamp: &str, key: &str) -> HeaderMap {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(timestamp.as_bytes());
    mac.update(PARTNER.as_bytes());
    mac.update(b"sid_request");
    let mut h = HeaderMap::new();
    h.insert(
        "response-timestamp",
        HeaderValue::from_str(timestamp).unwrap(),
    );
    h.insert(
        "response-signature",
        HeaderValue::from_str(&B64.encode(mac.finalize().into_bytes())).unwrap(),
    );
    h
}

fn clear(user_id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "status": "clear", "reason": null, "product": "biometric_kyc",
        "partner_params": { "job_id": "job_x", "user_id": user_id },
        "id_fields": { "full_name": "Amina Fatou Clearwater", "date_of_birth": "1992-08-22" },
    }))
    .unwrap()
}

fn result(status: &str, reason: &str, user_id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "status": status, "reason": reason, "id_fields": null,
        "partner_params": { "job_id": "job_x", "user_id": user_id },
    }))
    .unwrap()
}

#[tokio::test]
async fn start_sends_one_token_request_and_returns_the_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/token"))
        .and(header("smileid-partner-id", PARTNER))
        .and(header("smileid-api-key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "token": "jwt-abc" })))
        .expect(1)
        .mount(&server)
        .await;
    let reference = Uuid::new_v4();
    let job = provider(&server.uri())
        .start(&nin(), reference, "Ada Chinwe Obi")
        .await
        .unwrap();
    assert_eq!(job.session_token, "jwt-abc");
    assert_eq!(job.provider_ref, smile::user_id_for(reference));

    let sent = &server.received_requests().await.unwrap()[0];
    let body = String::from_utf8(sent.body.clone()).unwrap();
    assert!(body.contains("name=\"product\"\r\n\r\nbiometric_kyc"));
    assert!(body.contains(&smile::user_id_for(reference)));
    let payload_line = body
        .split("name=\"payload\"\r\n\r\n")
        .nth(1)
        .and_then(|rest| rest.split("\r\n").next())
        .unwrap();
    let payload: Value = serde_json::from_str(payload_line).unwrap();
    assert_eq!(payload["id_type"], "NIN_V2");
    assert_eq!(payload["id_number"], "12345678901");
    assert_eq!(payload["given_names"], "Ada Chinwe");
    assert_eq!(payload["last_name"], "Obi");
    assert_eq!(payload["consent"]["granted"], true);
}

#[tokio::test]
async fn start_failures_carry_no_detail_and_are_not_retried() {
    for status in [400, 401, 500] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v3/token"))
            .respond_with(ResponseTemplate::new(status).set_body_string("nin 12345678901 bad"))
            .expect(1)
            .mount(&server)
            .await;
        let err = provider(&server.uri())
            .start(&nin(), Uuid::new_v4(), "Ada Obi")
            .await
            .unwrap_err();
        assert_eq!(err, IdentityError::Unavailable);
        assert!(!format!("{err:?}{err}").contains("1234"));
    }
}

#[tokio::test]
async fn a_reply_without_a_token_is_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v3/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "nope": 1 })))
        .mount(&server)
        .await;
    let err = provider(&server.uri())
        .start(&nin(), Uuid::new_v4(), "Ada Obi")
        .await
        .unwrap_err();
    assert_eq!(err, IdentityError::Unavailable);
}

#[test]
fn a_clear_result_carries_the_verified_name_and_birth_date() {
    let p = provider("http://unused.invalid");
    let ev = p
        .parse_webhook(&signed("2026-03-02T16:24:35Z", KEY), &clear("user_a"))
        .unwrap();
    assert_eq!(ev.provider_ref, "user_a");
    match ev.outcome {
        Outcome::Passed(v) => {
            assert_eq!(v.full_name, "Amina Fatou Clearwater");
            assert_eq!(v.date_of_birth.to_string(), "1992-08-22");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn block_and_attention_map_conservatively() {
    let p = provider("http://unused.invalid");
    let h = signed("t", KEY);
    let map = |status, reason| {
        p.parse_webhook(&h, &result(status, reason, "user_a"))
            .unwrap()
            .outcome
    };
    assert_eq!(
        map("block", "identifier_not_found"),
        Outcome::Failed(FailReason::NotFound)
    );
    assert_eq!(
        map("block", "face_verification_failed"),
        Outcome::Failed(FailReason::FaceMismatch)
    );
    assert_eq!(
        map("block", "spoof_detected"),
        Outcome::Failed(FailReason::NotAlive)
    );
    assert_eq!(map("block", "high_risk"), Outcome::Unsure);
    assert_eq!(map("block", "something_new"), Outcome::Unsure);
    assert_eq!(map("attention", "medium_risk"), Outcome::Unsure);
}

#[test]
fn an_error_status_or_a_clear_without_fields_is_not_a_verdict() {
    let p = provider("http://unused.invalid");
    let h = signed("t", KEY);
    for body in [
        result("error", "service_unavailable", "user_a"),
        result("clear", "", "user_a"),
        result("whatever", "", "user_a"),
        b"not json".to_vec(),
    ] {
        assert_eq!(
            p.parse_webhook(&h, &body).unwrap_err(),
            IdentityError::Malformed
        );
    }
}

#[test]
fn the_signature_is_checked_before_the_body_is_read() {
    let p = provider("http://unused.invalid");
    // Wrong key, missing headers, bad base64, a changed timestamp: all refused, even for
    // a body that would not parse.
    let mut tampered = signed("t1", KEY);
    tampered.insert("response-timestamp", HeaderValue::from_static("t2"));
    let mut bad_b64 = signed("t", KEY);
    bad_b64.insert("response-signature", HeaderValue::from_static("%%%"));
    for h in [
        signed("t", "another-key"),
        HeaderMap::new(),
        tampered,
        bad_b64,
    ] {
        assert_eq!(
            p.parse_webhook(&h, b"not json").unwrap_err(),
            IdentityError::InvalidSignature
        );
    }
}

#[tokio::test]
async fn fetch_has_no_result_to_give() {
    assert_eq!(
        provider("http://unused.invalid").fetch("user_a").await,
        Ok(None)
    );
}

#[test]
fn config_debug_hides_the_key() {
    let p = format!("{:?}", provider("http://x.invalid").sdk_params());
    assert!(p.contains("NIN_V2") && !p.contains(KEY));
}
