//! PaystackProvider against a local mock server (wiremock). No real network.
//!
//! The fixtures follow Paystack's public documentation. Shapes it does not document are
//! marked "unconfirmed" in `src/payments/paystack.rs` and in the fixtures here; every one of
//! them must be checked against a Paystack sandbox before real money.

mod dead_port;

use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue};
use dead_port::DeadPort;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use server::payments::{
    InitiatePayment, PaymentProvider, PaymentStatus, PayoutRequest, PayoutState, ProviderError,
    RefundRequest, WebhookEvent,
    mono::Secret,
    payment_url_allowed,
    paystack::{METHOD, PAYSTACK_NAME, PaystackConfig, PaystackProvider},
};
use sha2::Sha512;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, path_regex, query_param},
};

const SECRET_KEY: &str = "sk_test_9f8e7d6c5b4a32100000";
const ACCOUNT_NUMBER: &str = "0987654321";
const PAY_REF: &str = "CK1ABCDEFGHIJKLMNOPQR";
const PAYOUT_REF: &str = "PO1ABCDEFGHIJKLMNOPQRS";

fn config(base_url: &str, live: bool) -> PaystackConfig {
    PaystackConfig {
        secret_key: Secret::new(SECRET_KEY),
        base_url: base_url.to_string(),
        live,
        payer_email_domain: "pay.example.ng".into(),
        connect_timeout: Duration::from_secs(5),
        timeout: Duration::from_secs(15),
        insecure_http_for_tests: true,
    }
}

fn provider(server: &MockServer) -> PaystackProvider {
    PaystackProvider::new(config(&server.uri(), false)).unwrap()
}

fn initiate_req(email: Option<&str>) -> InitiatePayment {
    InitiatePayment {
        reference: PAY_REF.into(),
        amount_kobo: 1_050_000,
        description: "Marketplace order".into(),
        customer_name: "Ada Obi".into(),
        customer_email: email.map(str::to_string),
        redirect_url: "https://shop.example/checkout/return".into(),
    }
}

fn payout_req() -> PayoutRequest {
    PayoutRequest {
        reference: PAYOUT_REF.into(),
        amount_kobo: 900_000,
        bank_code: "058".into(),
        account_number: ACCOUNT_NUMBER.into(),
        narration: "Marketplace payout".into(),
    }
}

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"status": true, "message": "ok", "data": data}))
}

fn refused(status: u16, message: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({"status": false, "message": message}))
}

/// Every request the server saw, as (method, path).
async fn seen(server: &MockServer) -> Vec<(String, String)> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| (r.method.to_string(), r.url.path().to_string()))
        .collect()
}

// ---- collection ----

#[tokio::test]
async fn initiate_sends_only_what_is_needed_and_returns_the_hosted_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/transaction/initialize"))
        .and(header(
            "authorization",
            format!("Bearer {SECRET_KEY}").as_str(),
        ))
        // No name; the buyer has no email, so a synthetic one on our own domain.
        .and(body_json(json!({
            "email": "ck1abcdefghijklmnopqr@pay.example.ng",
            "amount": 1_050_000,
            "currency": "NGN",
            "reference": PAY_REF,
            "callback_url": "https://shop.example/checkout/return",
        })))
        .respond_with(ok(json!({
            "authorization_url": "https://checkout.paystack.com/abc123",
            "access_code": "abc123",
            "reference": PAY_REF,
        })))
        .expect(1)
        .mount(&server)
        .await;
    let got = provider(&server)
        .initiate(initiate_req(None))
        .await
        .unwrap();
    assert_eq!(got.provider_payment_id, "abc123");
    assert_eq!(got.payment_url, "https://checkout.paystack.com/abc123");
    // The hosted page is on a host the provider says it serves from.
    let p = provider(&server);
    assert!(payment_url_allowed(&got.payment_url, p.payment_url_hosts()));
    assert!(!payment_url_allowed(
        "https://paystack.com.evil.test/x",
        p.payment_url_hosts()
    ));
    assert_eq!(p.name(), PAYSTACK_NAME);
    assert_eq!(p.methods(), [METHOD]);
}

#[tokio::test]
async fn a_buyers_own_email_is_used_when_there_is_one_and_a_bad_one_is_not() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/transaction/initialize"))
        .respond_with(ok(
            json!({"authorization_url": "https://checkout.paystack.com/x", "access_code": "x"}),
        ))
        .mount(&server)
        .await;
    let p = provider(&server);
    p.initiate(initiate_req(Some("ada@example.test")))
        .await
        .unwrap();
    p.initiate(initiate_req(Some("not an email")))
        .await
        .unwrap();
    let bodies: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    assert_eq!(bodies[0]["email"], "ada@example.test");
    assert_eq!(bodies[1]["email"], "ck1abcdefghijklmnopqr@pay.example.ng");
    for b in &bodies {
        assert!(
            b.get("name").is_none()
                && b.get("customer_name").is_none()
                && !b.to_string().contains("Ada Obi")
        );
    }
}

#[tokio::test]
async fn initiate_failures_are_mapped_by_whether_anything_could_have_been_sent() {
    let server = MockServer::start().await;
    // A bad amount is refused here, before anything is sent.
    let mut req = initiate_req(None);
    req.amount_kobo = 0;
    assert!(matches!(
        provider(&server).initiate(req).await,
        Err(ProviderError::Rejected { .. })
    ));
    assert!(seen(&server).await.is_empty());

    for (status, want_timeout) in [(500, true), (502, true)] {
        let s = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(refused(status, "boom"))
            .mount(&s)
            .await;
        let r = provider(&s).initiate(initiate_req(None)).await;
        assert_eq!(
            matches!(r, Err(ProviderError::Timeout)),
            want_timeout,
            "{status}: {r:?}"
        );
    }
    // A refusal is a refusal; a rate limit is "not processed, try later".
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(refused(400, "Duplicate Transaction Reference"))
        .mount(&s)
        .await;
    assert!(matches!(
        provider(&s).initiate(initiate_req(None)).await,
        Err(ProviderError::Rejected { .. })
    ));
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(refused(429, "slow down"))
        .mount(&s)
        .await;
    assert!(matches!(
        provider(&s).initiate(initiate_req(None)).await,
        Err(ProviderError::Unavailable(_))
    ));
    // A 2xx that says status false, and a 2xx we cannot read: refusal, and unknown outcome.
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"status": false, "message": "no"})),
        )
        .mount(&s)
        .await;
    assert!(matches!(
        provider(&s).initiate(initiate_req(None)).await,
        Err(ProviderError::Rejected { .. })
    ));
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
        .mount(&s)
        .await;
    assert!(matches!(
        provider(&s).initiate(initiate_req(None)).await,
        Err(ProviderError::Timeout)
    ));
    // Nothing listening: nothing was sent.
    let dead = DeadPort::new();
    let p = PaystackProvider::new(config(&format!("http://{}", dead.addr), false)).unwrap();
    assert!(matches!(
        p.initiate(initiate_req(None)).await,
        Err(ProviderError::Unavailable(_))
    ));
}

#[tokio::test]
async fn verify_maps_every_status_and_never_invents_a_terminal_one() {
    let cases: [(&str, Value, PaymentStatus); 7] = [
        (
            "success",
            json!({"status": "success", "amount": 1_050_000, "fees": 15_750, "currency": "NGN", "reference": PAY_REF}),
            PaymentStatus::Successful {
                amount_kobo: 1_050_000,
                fee_kobo: 15_750,
                currency: "NGN".into(),
                held: false,
                reference: PAY_REF.into(),
            },
        ),
        ("failed", json!({"status": "failed"}), PaymentStatus::Failed),
        (
            "reversed",
            json!({"status": "reversed"}),
            PaymentStatus::Failed,
        ),
        (
            "abandoned",
            json!({"status": "abandoned"}),
            PaymentStatus::Pending,
        ),
        (
            "ongoing",
            json!({"status": "ongoing"}),
            PaymentStatus::Pending,
        ),
        (
            "queued",
            json!({"status": "queued"}),
            PaymentStatus::Pending,
        ),
        (
            "something new",
            json!({"status": "a-status-we-never-saw"}),
            PaymentStatus::Pending,
        ),
    ];
    for (name, data, want) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/transaction/verify/{PAY_REF}")))
            .and(header(
                "authorization",
                format!("Bearer {SECRET_KEY}").as_str(),
            ))
            .respond_with(ok(data))
            .mount(&server)
            .await;
        assert_eq!(
            provider(&server).verify_payment(PAY_REF).await.unwrap(),
            want,
            "{name}"
        );
    }
    // A fee Paystack does not report is 0, and the currency defaults to NGN.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok(json!({"status": "success", "amount": "500000"})))
        .mount(&server)
        .await;
    match provider(&server).verify_payment(PAY_REF).await.unwrap() {
        PaymentStatus::Successful {
            amount_kobo,
            fee_kobo,
            currency,
            ..
        } => {
            assert_eq!(
                (amount_kobo, fee_kobo, currency.as_str()),
                (500_000, 0, "NGN")
            );
        }
        other => panic!("{other:?}"),
    }
    // A success without an amount cannot be trusted.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok(json!({"status": "success"})))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).verify_payment(PAY_REF).await,
        Err(ProviderError::Timeout)
    ));
}

#[tokio::test]
async fn a_payment_paystack_has_never_seen_is_pending_and_an_outage_is_not_an_answer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(refused(404, "Transaction reference not found"))
        .mount(&server)
        .await;
    assert_eq!(
        provider(&server).verify_payment(PAY_REF).await.unwrap(),
        PaymentStatus::Pending
    );
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(refused(503, "down"))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).verify_payment(PAY_REF).await,
        Err(ProviderError::Unavailable(_))
    ));
}

// ---- payouts ----

#[tokio::test]
async fn a_payout_makes_a_recipient_then_one_transfer_under_a_lowercase_reference() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/transferrecipient"))
        .and(body_json(json!({
            "type": "nuban",
            "name": "Marketplace seller",
            "account_number": ACCOUNT_NUMBER,
            "bank_code": "058",
            "currency": "NGN",
        })))
        .respond_with(ok(json!({"recipient_code": "RCP_abc"})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/transfer"))
        .and(body_json(json!({
            "source": "balance",
            "amount": 900_000,
            "recipient": "RCP_abc",
            "reference": PAYOUT_REF.to_ascii_lowercase(),
            "reason": "Marketplace payout",
        })))
        .respond_with(ok(json!({"transfer_code": "TRF_xyz", "status": "pending", "reference": PAYOUT_REF.to_ascii_lowercase()})))
        .expect(1)
        .mount(&server)
        .await;
    let handle = provider(&server).create_payout(payout_req()).await.unwrap();
    assert_eq!(handle.provider_payout_id, "TRF_xyz");
    // The paystack reference rule: lowercase, digits, 16 to 50 characters.
    let r = PAYOUT_REF.to_ascii_lowercase();
    assert!(
        (16..=50).contains(&r.len())
            && r.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    );
}

#[tokio::test]
async fn a_failure_before_any_transfer_is_safe_to_retry_and_one_after_it_is_not() {
    // The recipient step fails: no money moved, so it is "try again", and no transfer is sent.
    for status in [500, 429] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/transferrecipient"))
            .respond_with(refused(status, "x"))
            .mount(&server)
            .await;
        let r = provider(&server).create_payout(payout_req()).await;
        assert!(
            matches!(r, Err(ProviderError::Unavailable(_))),
            "{status}: {r:?}"
        );
        assert!(
            seen(&server)
                .await
                .iter()
                .all(|(_, p)| p == "/transferrecipient")
        );
    }
    // A bad account is a refusal, still before any transfer.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/transferrecipient"))
        .respond_with(refused(422, "Could not resolve account name"))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).create_payout(payout_req()).await,
        Err(ProviderError::Rejected { .. })
    ));
    // The transfer step fails after sending: the outcome is unknown, never "not done".
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/transferrecipient"))
        .respond_with(ok(json!({"recipient_code": "RCP_abc"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/transfer"))
        .respond_with(refused(500, "x"))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).create_payout(payout_req()).await,
        Err(ProviderError::Timeout)
    ));
    // And it was sent exactly once.
    let transfers = seen(&server)
        .await
        .iter()
        .filter(|(_, p)| p == "/transfer")
        .count();
    assert_eq!(transfers, 1);
    // A transfer Paystack reports as failed at once is a refusal.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/transferrecipient"))
        .respond_with(ok(json!({"recipient_code": "RCP_abc"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/transfer"))
        .respond_with(ok(json!({"transfer_code": "TRF_1", "status": "failed"})))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).create_payout(payout_req()).await,
        Err(ProviderError::Rejected { .. })
    ));
}

#[tokio::test]
async fn a_duplicate_reference_finds_the_earlier_transfer_instead_of_paying_twice() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/transferrecipient"))
        .respond_with(ok(json!({"recipient_code": "RCP_abc"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/transfer"))
        .respond_with(refused(400, "Transfer with this reference already exists"))
        .mount(&server)
        .await;
    let lower = PAYOUT_REF.to_ascii_lowercase();
    Mock::given(method("GET"))
        .and(path(format!("/transfer/verify/{lower}")))
        .respond_with(ok(
            json!({"transfer_code": "TRF_earlier", "status": "success"}),
        ))
        .mount(&server)
        .await;
    let handle = provider(&server).create_payout(payout_req()).await.unwrap();
    assert_eq!(handle.provider_payout_id, "TRF_earlier");

    // A duplicate-looking refusal with nothing found is just a refusal.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/transferrecipient"))
        .respond_with(ok(json!({"recipient_code": "RCP_abc"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/transfer"))
        .respond_with(refused(400, "Duplicate Transfer Reference"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(refused(404, "Transfer not found"))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).create_payout(payout_req()).await,
        Err(ProviderError::Rejected { .. })
    ));
}

#[tokio::test]
async fn payout_status_maps_each_state_by_our_uppercase_reference() {
    let lower = PAYOUT_REF.to_ascii_lowercase();
    let cases: [(&str, Value, PayoutState); 7] = [
        (
            "success with a fee",
            json!({"status": "success", "amount": 900_000, "fee_charged": 5_000}),
            PayoutState::Successful {
                fee_kobo: 5_000,
                amount_kobo: Some(900_000),
            },
        ),
        (
            "success, no fee reported (unconfirmed shape)",
            json!({"status": "success", "amount": 900_000}),
            PayoutState::Successful {
                fee_kobo: 0,
                amount_kobo: Some(900_000),
            },
        ),
        (
            "failed",
            json!({"status": "failed", "message": "for 0987654321"}),
            PayoutState::Failed {
                reason: "paystack_transfer_failed".into(),
            },
        ),
        (
            "reversed",
            json!({"status": "reversed"}),
            PayoutState::Reversed {
                reason: "paystack_transfer_reversed".into(),
            },
        ),
        (
            "pending",
            json!({"status": "pending"}),
            PayoutState::Pending,
        ),
        ("otp", json!({"status": "otp"}), PayoutState::Pending),
        (
            "unknown",
            json!({"status": "who-knows"}),
            PayoutState::Pending,
        ),
    ];
    for (name, data, want) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/transfer/verify/{lower}")))
            .respond_with(ok(data))
            .mount(&server)
            .await;
        assert_eq!(
            provider(&server)
                .payout_status(PAYOUT_REF, None)
                .await
                .unwrap(),
            want,
            "{name}"
        );
    }
    // A failed transfer's text can echo the account: only a fixed reason is ever kept.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok(json!({"status": "failed", "message": "for 0987654321"})))
        .mount(&server)
        .await;
    let s = provider(&server)
        .payout_status(PAYOUT_REF, None)
        .await
        .unwrap();
    assert!(!format!("{s:?}").contains("0987654321"));
}

#[tokio::test]
async fn find_payout_is_none_only_when_paystack_says_it_is_not_there() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(refused(404, "Transfer not found"))
        .mount(&server)
        .await;
    assert_eq!(
        provider(&server).find_payout(PAYOUT_REF).await.unwrap(),
        None
    );
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(refused(500, "x"))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).find_payout(PAYOUT_REF).await,
        Err(ProviderError::Unavailable(_))
    ));
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok(json!({"id": 98765, "status": "pending"})))
        .mount(&server)
        .await;
    // An id alone will do when there is no transfer code.
    assert_eq!(
        provider(&server)
            .find_payout(PAYOUT_REF)
            .await
            .unwrap()
            .unwrap()
            .provider_payout_id,
        "98765"
    );
}

#[tokio::test]
async fn an_otp_transfer_is_finished_with_the_code_and_a_wrong_code_is_refused() {
    let lower = PAYOUT_REF.to_ascii_lowercase();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/transfer/verify/{lower}")))
        .respond_with(ok(json!({"transfer_code": "TRF_otp", "status": "otp"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/transfer/finalize_transfer"))
        .and(body_json(
            json!({"transfer_code": "TRF_otp", "otp": "123456"}),
        ))
        .respond_with(ok(json!({"status": "success"})))
        .expect(1)
        .mount(&server)
        .await;
    provider(&server)
        .finalize_payout(PAYOUT_REF, "123456")
        .await
        .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok(json!({"transfer_code": "TRF_otp"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/transfer/finalize_transfer"))
        .respond_with(refused(400, "Invalid OTP"))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server)
            .finalize_payout(PAYOUT_REF, "000000")
            .await,
        Err(ProviderError::Rejected { .. })
    ));
}

// ---- refunds and account names ----

fn refund_req() -> RefundRequest {
    RefundRequest {
        reference: "RF1ABCDEFGHIJKLMNOPQRS".into(),
        payment_reference: PAY_REF.into(),
        amount_kobo: 250_000,
        reason: "buyer cancelled for Ada Obi".into(),
    }
}

#[tokio::test]
async fn a_refund_is_created_once_with_our_reference_and_no_reason_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/refund"))
        .and(query_param("reference", PAY_REF))
        .respond_with(ok(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/refund"))
        .and(body_json(json!({
            "transaction": PAY_REF,
            "amount": 250_000,
            "currency": "NGN",
            "merchant_note": "RF1ABCDEFGHIJKLMNOPQRS",
            "customer_note": "Marketplace order refund",
        })))
        .respond_with(ok(json!({"id": 4242, "status": "pending"})))
        .expect(1)
        .mount(&server)
        .await;
    let h = provider(&server).refund(refund_req()).await.unwrap();
    assert_eq!(h.provider_refund_id, "4242");
    // The free-text reason (it can name a person) is not sent.
    let body = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect::<String>();
    assert!(!body.contains("Ada"), "{body}");
}

#[tokio::test]
async fn a_repeated_refund_reuses_the_earlier_one_instead_of_refunding_twice() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/refund"))
        .respond_with(ok(json!([
            {"id": 1, "status": "failed", "merchant_note": "RF1ABCDEFGHIJKLMNOPQRS"},
            {"id": 2, "status": "pending", "merchant_note": "SOMETHING-ELSE"},
            {"id": 3, "status": "processing", "merchant_note": "RF1ABCDEFGHIJKLMNOPQRS"},
        ])))
        .mount(&server)
        .await;
    let h = provider(&server).refund(refund_req()).await.unwrap();
    assert_eq!(
        h.provider_refund_id, "3",
        "ours, and not the failed attempt"
    );
    assert!(
        seen(&server).await.iter().all(|(m, _)| m == "GET"),
        "nothing was created"
    );
}

#[tokio::test]
async fn a_refund_does_not_create_blind_when_it_could_not_look() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/refund"))
        .respond_with(refused(503, "x"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/refund"))
        .respond_with(ok(json!({"id": 9})))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).refund(refund_req()).await,
        Err(ProviderError::Unavailable(_))
    ));
    assert!(seen(&server).await.iter().all(|(m, _)| m == "GET"));
    // A refund Paystack says failed, and one that fails after sending.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/refund"))
        .respond_with(ok(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/refund"))
        .respond_with(ok(json!({"id": 9, "status": "failed"})))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).refund(refund_req()).await,
        Err(ProviderError::Rejected { .. })
    ));
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/refund"))
        .respond_with(ok(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/refund"))
        .respond_with(refused(500, "x"))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server).refund(refund_req()).await,
        Err(ProviderError::Timeout)
    ));
}

#[tokio::test]
async fn an_account_name_is_resolved_and_a_bad_one_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/bank/resolve"))
        .and(query_param("account_number", ACCOUNT_NUMBER))
        .and(query_param("bank_code", "058"))
        .respond_with(ok(
            json!({"account_name": "  ADA OBI  ", "account_number": ACCOUNT_NUMBER}),
        ))
        .mount(&server)
        .await;
    assert_eq!(
        provider(&server)
            .resolve_account("058", ACCOUNT_NUMBER)
            .await
            .unwrap(),
        "ADA OBI"
    );
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(refused(422, "Could not resolve account name"))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server)
            .resolve_account("058", ACCOUNT_NUMBER)
            .await,
        Err(ProviderError::Rejected { .. })
    ));
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok(json!({"account_name": ""})))
        .mount(&server)
        .await;
    assert!(matches!(
        provider(&server)
            .resolve_account("058", ACCOUNT_NUMBER)
            .await,
        Err(ProviderError::Inconclusive)
    ));
}

// ---- webhooks ----

fn signed(body: &str, key: &str) -> HeaderMap {
    let mut mac = Hmac::<Sha512>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let sig: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut h = HeaderMap::new();
    h.insert("x-paystack-signature", HeaderValue::from_str(&sig).unwrap());
    h
}

fn hook(live: bool) -> PaystackProvider {
    // The webhook path makes no call: any address will do.
    PaystackProvider::new(config("http://127.0.0.1:9", live)).unwrap()
}

#[tokio::test]
async fn webhooks_need_a_valid_signature_of_the_exact_body() {
    let p = hook(false);
    let body = json!({"event": "charge.success", "data": {"reference": PAY_REF, "domain": "test"}})
        .to_string();
    assert!(matches!(
        p.verify_webhook(&signed(&body, SECRET_KEY), body.as_bytes()),
        Ok(WebhookEvent::Payment { .. })
    ));
    // A capital-letter signature is the same signature.
    let mut upper = HeaderMap::new();
    upper.insert(
        "x-paystack-signature",
        HeaderValue::from_str(
            &signed(&body, SECRET_KEY)["x-paystack-signature"]
                .to_str()
                .unwrap()
                .to_uppercase(),
        )
        .unwrap(),
    );
    assert!(p.verify_webhook(&upper, body.as_bytes()).is_ok());
    // No header, another key, a tampered body, junk, empty.
    assert!(matches!(
        p.verify_webhook(&HeaderMap::new(), body.as_bytes()),
        Err(ProviderError::InvalidSignature)
    ));
    assert!(matches!(
        p.verify_webhook(
            &signed(&body, "sk_test_another_key_entirely"),
            body.as_bytes()
        ),
        Err(ProviderError::InvalidSignature)
    ));
    let tampered = body.replace(PAY_REF, "CK9ZZZZZZZZZZZZZZZZZZ");
    assert!(matches!(
        p.verify_webhook(&signed(&body, SECRET_KEY), tampered.as_bytes()),
        Err(ProviderError::InvalidSignature)
    ));
    let mut junk = HeaderMap::new();
    junk.insert("x-paystack-signature", HeaderValue::from_static("zz"));
    assert!(matches!(
        p.verify_webhook(&junk, body.as_bytes()),
        Err(ProviderError::InvalidSignature)
    ));
}

#[tokio::test]
async fn webhook_events_become_payment_and_payout_events_with_our_uppercase_references() {
    let p = hook(false);
    let lower = PAYOUT_REF.to_ascii_lowercase();
    for event in ["transfer.success", "transfer.failed", "transfer.reversed"] {
        let body =
            json!({"event": event, "data": {"reference": lower, "domain": "test", "id": 77}})
                .to_string();
        match p
            .verify_webhook(&signed(&body, SECRET_KEY), body.as_bytes())
            .unwrap()
        {
            WebhookEvent::Payout {
                event_id,
                reference,
            } => {
                assert_eq!(reference, PAYOUT_REF, "{event}");
                assert_eq!(event_id, format!("{event}:{PAYOUT_REF}"));
            }
            other => panic!("{event}: {other:?}"),
        }
    }
    let body = json!({"event": "charge.success", "data": {"reference": PAY_REF, "domain": "test"}})
        .to_string();
    match p
        .verify_webhook(&signed(&body, SECRET_KEY), body.as_bytes())
        .unwrap()
    {
        WebhookEvent::Payment {
            event_id,
            reference,
        } => {
            assert_eq!(
                (reference.as_str(), event_id.as_str()),
                (PAY_REF, format!("charge.success:{PAY_REF}").as_str())
            );
        }
        other => panic!("{other:?}"),
    }
    // Events we do not act on, and ones missing what we need, are ignored.
    for body in [
        json!({"event": "refund.processed", "data": {"id": 5, "domain": "test"}}),
        json!({"event": "customeridentification.success", "data": {}}),
        json!({"event": "charge.success", "data": {"domain": "test"}}),
        json!({"event": "transfer.success", "data": {"reference": "  ", "domain": "test"}}),
    ] {
        let body = body.to_string();
        assert!(
            matches!(
                p.verify_webhook(&signed(&body, SECRET_KEY), body.as_bytes()),
                Ok(WebhookEvent::Ignored { .. })
            ),
            "{body}"
        );
    }
    // Signed, but not an event.
    for body in ["not json", "{}", r#"{"data":{}}"#] {
        assert!(
            matches!(
                p.verify_webhook(&signed(body, SECRET_KEY), body.as_bytes()),
                Err(ProviderError::Rejected { .. })
            ),
            "{body}"
        );
    }
}

#[tokio::test]
async fn an_event_from_the_other_mode_is_ignored() {
    let test_event =
        json!({"event": "charge.success", "data": {"reference": PAY_REF, "domain": "test"}})
            .to_string();
    let live_event =
        json!({"event": "charge.success", "data": {"reference": PAY_REF, "domain": "live"}})
            .to_string();
    // A live server ignores a test event, and a test server ignores a live one.
    assert!(matches!(
        hook(true).verify_webhook(&signed(&test_event, SECRET_KEY), test_event.as_bytes()),
        Ok(WebhookEvent::Ignored { .. })
    ));
    assert!(matches!(
        hook(false).verify_webhook(&signed(&live_event, SECRET_KEY), live_event.as_bytes()),
        Ok(WebhookEvent::Ignored { .. })
    ));
    assert!(matches!(
        hook(true).verify_webhook(&signed(&live_event, SECRET_KEY), live_event.as_bytes()),
        Ok(WebhookEvent::Payment { .. })
    ));
}

// ---- secrets ----

#[tokio::test]
async fn the_secret_key_never_appears_in_debug_or_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("/.*"))
        .respond_with(refused(401, "Invalid key sk_test_9f8e7d6c5b4a32100000"))
        .mount(&server)
        .await;
    let p = provider(&server);
    assert!(!format!("{p:?}").contains(SECRET_KEY));
    let err = p.verify_payment(PAY_REF).await.unwrap_err();
    for text in [format!("{err:?}"), err.to_string()] {
        assert!(!text.contains(SECRET_KEY), "{text}");
    }
    // The key header is sent but marked sensitive, and a payout error holds no account number.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(refused(422, "bad account 0987654321"))
        .mount(&server)
        .await;
    let err = provider(&server)
        .create_payout(payout_req())
        .await
        .unwrap_err();
    assert!(!format!("{err:?}").contains("0987654321"));
}
