//! The Termii SMS adapter against a local mock server (wiremock). No real network.
//! The request shape is the one in Termii's messaging API documentation.

use std::time::Duration;

mod dead_port;

use dead_port::DeadPort;
use serde_json::{Value, json};
use server::{
    auth::{
        otp::OtpSender,
        sms::{SmsError, SmsOtpSender, SmsProvider, code_text},
        sms_termii::{MAX_RESPONSE_BODY, TermiiConfig, TermiiSms},
    },
    error::AppError,
    payments::mono::Secret,
};
use std::sync::Arc;
use tracing_test::traced_test;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const API_KEY: &str = "TLkey-never-log-this-0123456789";
const SENDER: &str = "StockApp";
const PHONE: &str = "+2348031234567";
const TEXT: &str = "Your code is 482913. It expires in 10 minutes. Never share it.";

fn config(base_url: String) -> TermiiConfig {
    TermiiConfig {
        api_key: Secret::new(API_KEY),
        sender_id: SENDER.into(),
        base_url,
        connect_timeout: Duration::from_millis(500),
        timeout: Duration::from_millis(400),
        insecure_http_for_tests: true,
    }
}

fn adapter(server: &MockServer) -> TermiiSms {
    TermiiSms::new(config(server.uri())).unwrap()
}

fn ok_body() -> Value {
    json!({
        "code": "ok",
        "balance": 9800.0,
        "message_id": "9122821270554876574",
        "message": "Successfully Sent",
        "user": "Stock App",
        "message_id_str": "9122821270554876574"
    })
}

#[tokio::test]
async fn the_request_is_exactly_what_termii_documents() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/sms/send"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .expect(1)
        .mount(&server)
        .await;
    adapter(&server).send(PHONE, TEXT).await.unwrap();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(
        req.headers.get("content-type").unwrap().to_str().unwrap(),
        "application/json"
    );
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    // Exactly these fields: the number without its plus sign, the do-not-disturb route (the
    // only one that reliably delivers a code), a plain text message.
    assert_eq!(
        body,
        json!({
            "api_key": API_KEY,
            "to": "2348031234567",
            "from": SENDER,
            "sms": TEXT,
            "type": "plain",
            "channel": "dnd",
        })
    );
}

#[tokio::test]
async fn statuses_map_to_rejected_or_unavailable() {
    for (status, expected) in [
        (400, SmsError::Rejected),
        (401, SmsError::Rejected),
        (403, SmsError::Rejected),
        (404, SmsError::Rejected),
        (422, SmsError::Rejected),
        (429, SmsError::Unavailable),
        (500, SmsError::Unavailable),
        (502, SmsError::Unavailable),
        (503, SmsError::Unavailable),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status).set_body_json(json!({ "message": "provider text" })),
            )
            .mount(&server)
            .await;
        assert_eq!(
            adapter(&server).send(PHONE, TEXT).await,
            Err(expected),
            "status {status}"
        );
    }
}

#[tokio::test]
async fn a_200_that_is_not_ok_or_not_json_is_not_a_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "code": "error", "message": "Insufficient balance"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    assert_eq!(
        adapter(&server).send(PHONE, TEXT).await,
        Err(SmsError::Rejected)
    );

    // Not JSON at all: we cannot tell whether it went, so it counts as unavailable.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>maintenance</html>"))
        .mount(&server)
        .await;
    assert_eq!(
        adapter(&server).send(PHONE, TEXT).await,
        Err(SmsError::Unavailable)
    );
}

#[tokio::test]
async fn slow_oversized_and_refused_are_unavailable() {
    // Slower than the timeout.
    let slow = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(ok_body())
                .set_delay(Duration::from_secs(2)),
        )
        .mount(&slow)
        .await;
    assert_eq!(
        adapter(&slow).send(PHONE, TEXT).await,
        Err(SmsError::Unavailable)
    );

    // A body over the cap.
    let big = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(MAX_RESPONSE_BODY + 1)))
        .mount(&big)
        .await;
    assert_eq!(
        adapter(&big).send(PHONE, TEXT).await,
        Err(SmsError::Unavailable)
    );

    // Nothing listening.
    let dead = DeadPort::new();
    assert_eq!(
        TermiiSms::new(config(format!("http://{}", dead.addr)))
            .unwrap()
            .send(PHONE, TEXT)
            .await,
        Err(SmsError::Unavailable)
    );
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = MockServer::start().await;
    let elsewhere = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/steal", elsewhere.uri())),
        )
        .mount(&server)
        .await;
    assert!(adapter(&server).send(PHONE, TEXT).await.is_err());
    assert!(
        elsewhere.received_requests().await.unwrap().is_empty(),
        "the api key must not follow a redirect"
    );
}

#[traced_test]
#[tokio::test]
async fn phone_code_key_and_provider_text_never_reach_logs_or_errors() {
    for response in [
        ResponseTemplate::new(400)
            .set_body_json(json!({ "message": "Invalid number 2348031234567" })),
        ResponseTemplate::new(503).set_body_string("SECRET-PROVIDER-TEXT"),
        ResponseTemplate::new(200)
            .set_body_json(json!({ "code": "error", "message": "SECRET-PROVIDER-TEXT" })),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(response)
            .mount(&server)
            .await;
        let sender = SmsOtpSender::new(Arc::new(adapter(&server)));
        let err = sender.send(PHONE, "482913").await.unwrap_err();
        // Through the sender, the caller sees only the fixed 503 code.
        assert!(
            matches!(err, AppError::Unavailable("otp_unavailable")),
            "{err:?}"
        );
        let shown = format!("{err:?} {err}");
        assert!(!shown.contains("SECRET-PROVIDER-TEXT") && !shown.contains("8031234567"));
    }
    for needle in [
        "482913",
        "8031234567",
        API_KEY,
        "SECRET-PROVIDER-TEXT",
        "Invalid number",
    ] {
        assert!(!logs_contain(needle), "logs show {needle}");
    }
    assert!(logs_contain("sms provider"), "the failure is still logged");
}

#[tokio::test]
async fn the_sms_sender_sends_the_fixed_text_through_the_adapter() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let sender = SmsOtpSender::new(Arc::new(adapter(&server)));
    sender.send(PHONE, "482913").await.unwrap();
    let body: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["sms"], code_text("482913"));
    assert_eq!(body["to"], "2348031234567");
}

#[test]
fn the_key_is_never_printed_and_http_is_refused_outside_tests() {
    let cfg = config("https://api.ng.termii.com".into());
    assert!(!format!("{cfg:?}").contains(API_KEY));

    let mut insecure = config("http://api.example.test".into());
    insecure.insecure_http_for_tests = false;
    assert!(TermiiSms::new(insecure).is_err());
    let mut no_host = config("https://".into());
    no_host.insecure_http_for_tests = false;
    assert!(TermiiSms::new(no_host).is_err());
}
