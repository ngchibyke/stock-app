//! Paystack through the real routes: a checkout, the hosted page, a signed webhook that
//! settles the order, and the settings that choose Paystack. The Paystack API is a local mock.

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::http::Method;
use common::TestApp;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use server::payments::{
    ProviderRegistry,
    mono::Secret,
    paystack::{PaystackConfig, PaystackProvider},
};
use sha2::Sha512;
use sqlx::PgPool;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, path_regex},
};

const KEY: &str = "sk_test_flowflowflowflow0001";

fn registry(server: &MockServer) -> ProviderRegistry {
    let mut r = ProviderRegistry::new();
    r.register(Arc::new(
        PaystackProvider::new(PaystackConfig {
            secret_key: Secret::new(KEY),
            base_url: server.uri(),
            live: false,
            payer_email_domain: "pay.example.ng".into(),
            connect_timeout: Duration::from_secs(5),
            timeout: Duration::from_secs(15),
            insecure_http_for_tests: true,
        })
        .unwrap(),
    ));
    r
}

fn signature(body: &str) -> String {
    let mut mac = Hmac::<Sha512>::new_from_slice(KEY.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

async fn webhook(app: &TestApp, body: &str, sig: &str) -> common::TestResponse {
    app.send_raw(
        Method::POST,
        "/webhooks/paystack",
        &[
            ("x-paystack-signature", sig),
            ("content-type", "application/json"),
        ],
        body.as_bytes().to_vec(),
    )
    .await
}

/// The owner chooses Paystack for collection and payouts, as the go-live steps say.
async fn choose_paystack(app: &TestApp) {
    let owner = app.owner_token().await;
    let current = app
        .request(Method::GET, "/admin/settings", None, Some(&owner))
        .await;
    assert_eq!(current.status, 200, "{}", current.json);
    let res = app
        .request(
            Method::PATCH,
            "/admin/settings",
            Some(json!({
                "enabled_methods": ["paystack_checkout"],
                "payout_provider": "paystack",
                "expected_updated_at": current.json["updated_at"],
            })),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["enabled_methods"], json!(["paystack_checkout"]));
    assert_eq!(res.json["payout_provider"], "paystack");
}

#[sqlx::test]
async fn the_owner_can_choose_paystack_and_a_made_up_provider_is_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    choose_paystack(&app).await;
    let owner = app.owner_token().await;
    let current = app
        .request(Method::GET, "/admin/settings", None, Some(&owner))
        .await;
    let res = app
        .request(
            Method::PATCH,
            "/admin/settings",
            Some(json!({"payout_provider": "madeup", "expected_updated_at": current.json["updated_at"]})),
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
}

#[sqlx::test]
async fn a_checkout_through_paystack_opens_its_page_and_a_signed_webhook_marks_it_paid(
    pool: PgPool,
) {
    let server = MockServer::start().await;
    let mut app = TestApp::new(pool).await;
    // The store is set up first (its payout account is checked by the default provider); then
    // the owner switches the marketplace over to Paystack.
    let listing = app
        .make_active_store_with_product("08031110099", "Paystack Store", 10_000, 5)
        .await;
    app.use_providers(registry(&server));
    choose_paystack(&app).await;
    let (buyer_id, buyer) = app.sign_up_verified("08031110098").await;
    assert_eq!(
        app.set_cart(&buyer, listing.variant_id, 1).await.status,
        200
    );

    // Paystack initialises the payment and gives the hosted page.
    Mock::given(method("POST"))
        .and(path("/transaction/initialize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": true, "message": "ok",
            "data": {"authorization_url": "https://checkout.paystack.com/flow123", "access_code": "flow123", "reference": "x"}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let res = app.checkout(&buyer, "paystack_checkout").await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(
        res.json["payment_url"],
        "https://checkout.paystack.com/flow123"
    );
    let reference = res.json["reference"].as_str().unwrap().to_string();
    let total = res.json["total_kobo"].as_i64().unwrap();
    let provider: String =
        sqlx::query_scalar("SELECT provider FROM checkouts WHERE reference = $1")
            .bind(&reference)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(provider, "paystack");
    // Only what is needed went to Paystack: the order's reference and amount, no name or phone.
    let sent: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(sent["amount"], total);
    assert_eq!(sent["reference"], reference);
    let text = sent.to_string();
    assert!(
        !text.contains("08031110098") && !text.to_lowercase().contains("buyer"),
        "{text}"
    );

    // A webhook with a bad signature changes nothing.
    let event = json!({"event": "charge.success", "data": {"reference": reference, "domain": "test", "id": 1}}).to_string();
    assert_eq!(webhook(&app, &event, "deadbeef").await.status, 401);
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM checkouts WHERE reference = $1")
            .bind(&reference)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(status, "pending");

    // The signed one makes the server ask Paystack, and the answer decides.
    Mock::given(method("GET"))
        .and(path_regex(format!("^/transaction/verify/{reference}$")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": true, "message": "ok",
            "data": {"status": "success", "amount": total, "fees": 150, "currency": "NGN", "reference": reference}
        })))
        .mount(&server)
        .await;
    let res = webhook(&app, &event, &signature(&event)).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM checkouts WHERE reference = $1")
            .bind(&reference)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(status, "paid");
    let order_status: String =
        sqlx::query_scalar("SELECT status::text FROM orders WHERE buyer_id = $1")
            .bind(buyer_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(order_status, "paid");
    // The same event again is a duplicate, not a second payment.
    let again = webhook(&app, &event, &signature(&event)).await;
    assert_eq!(again.status, 200);
    assert_eq!(again.json["status"], "duplicate", "{}", again.json);
}

#[sqlx::test]
async fn a_webhook_that_says_paid_for_the_wrong_amount_does_not_pay(pool: PgPool) {
    let server = MockServer::start().await;
    let mut app = TestApp::new(pool).await;
    let listing = app
        .make_active_store_with_product("08031110097", "Paystack Store 2", 10_000, 5)
        .await;
    app.use_providers(registry(&server));
    choose_paystack(&app).await;
    let (_id, buyer) = app.sign_up_verified("08031110096").await;
    app.set_cart(&buyer, listing.variant_id, 1).await;
    Mock::given(method("POST"))
        .and(path("/transaction/initialize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": true, "data": {"authorization_url": "https://checkout.paystack.com/w", "access_code": "w"}
        })))
        .mount(&server)
        .await;
    let res = app.checkout(&buyer, "paystack_checkout").await;
    let reference = res.json["reference"].as_str().unwrap().to_string();
    let total = res.json["total_kobo"].as_i64().unwrap();
    // Paystack says success, but for less than the order's total.
    Mock::given(method("GET"))
        .and(path_regex(format!("^/transaction/verify/{reference}$")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": true, "data": {"status": "success", "amount": total - 100, "currency": "NGN", "reference": reference}
        })))
        .mount(&server)
        .await;
    let event = json!({"event": "charge.success", "data": {"reference": reference, "domain": "test", "id": 2}}).to_string();
    webhook(&app, &event, &signature(&event)).await;
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM checkouts WHERE reference = $1")
            .bind(&reference)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_ne!(
        status, "paid",
        "an amount that does not match is never accepted as paid"
    );
}
