//! ShipbubbleCourier against a local mock server (wiremock). No real network.
//!
//! Fixtures follow the sample payloads in the Shipbubble research
//! (`research-part3-delivery.md`, `research-t4-5.md`, docs.shipbubble.com,
//! 2026-10-31). Shapes the docs do not show (the FCT `state` values, the
//! `status.changed` webhook body, error bodies) are marked "assumed".

use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use server::{
    config::Config,
    couriers::{
        ContactAddress, CourierError, CourierProvider, Parcel, ParcelItem, ShipmentState,
        shipbubble::{
            AmountUnit, SIGNATURE_HEADER, Secret, ShipbubbleConfig, ShipbubbleCourier,
            from_sb_amount, to_sb_amount,
        },
    },
    delivery::area::AreaCouncil,
};
use sha2::Sha512;
use time::{Date, Month};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

const API_KEY: &str = "sb_sandbox_9f8e7d6c5b4a3210";

fn config(base_url: &str) -> ShipbubbleConfig {
    ShipbubbleConfig {
        api_key: Secret::new(API_KEY),
        base_url: format!("{base_url}/v1/shipping"),
        live: false,
        amount_unit: AmountUnit::Naira,
        connect_timeout: Duration::from_secs(5),
        timeout: Duration::from_secs(15),
        insecure_http_for_tests: true,
    }
}

fn courier(server: &MockServer) -> ShipbubbleCourier {
    ShipbubbleCourier::new(config(&server.uri())).expect("build courier")
}

fn address() -> ContactAddress {
    ContactAddress {
        name: "Ada Obi".into(),
        email: Some("ada@example.test".into()),
        phone: "08031234567".into(),
        address_line: "12 Zambezi Crescent".into(),
        area: AreaCouncil::Amac,
    }
}

fn parcel() -> Parcel {
    Parcel {
        items: vec![ParcelItem {
            name: "Blue kettle".into(),
            unit_weight_grams: 2,
            unit_amount_kobo: 500_000,
            quantity: 2,
        }],
        length_cm: 20,
        width_cm: 15,
        height_cm: 10,
        category_id: 3_035_980,
    }
}

fn sign(body: &[u8]) -> String {
    let mut mac = Hmac::<Sha512>::new_from_slice(API_KEY.as_bytes()).unwrap();
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn signed(body: &Value) -> (HeaderMap, Vec<u8>) {
    let raw = serde_json::to_vec(body).unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        SIGNATURE_HEADER,
        HeaderValue::from_str(&sign(&raw)).unwrap(),
    );
    (headers, raw)
}

fn validate_response(state: &str, state_code: &str) -> Value {
    // Documented sample (validate address), trimmed. The FCT state values are assumed.
    json!({
        "status": "success",
        "message": "Address validated successfully",
        "data": {
            "name": "Ada Obi",
            "email": "ada@example.test",
            "phone": "08031234567",
            "formatted_address": "12 Zambezi Crescent, Maitama, Abuja",
            "country": "Nigeria",
            "country_code": "NG",
            "city": "Abuja",
            "city_code": "ABJ",
            "state": state,
            "state_code": state_code,
            "postal_code": "900001",
            "latitude": 9.0765,
            "longitude": 7.3986,
            "address_code": 98794022
        }
    })
}

fn rates_response() -> Value {
    // Documented sample (fetch rates), trimmed: courier_id is a string for some
    // couriers and a number for others, amounts are naira numbers.
    json!({
        "status": "success",
        "message": "Rates fetched successfully",
        "data": {
            "request_token": "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0",
            "couriers": [
                {
                    "courier_id": "dellyman",
                    "courier_name": "Dellyman",
                    "service_code": "dellyman",
                    "currency": "NGN",
                    "rate_card_amount": 2785,
                    "total": 2900.5,
                    "delivery_eta": "Within 24 hours"
                },
                {
                    "courier_id": 1,
                    "courier_name": "Routely",
                    "service_code": "routely",
                    "currency": "NGN",
                    "rate_card_amount": 12451.04,
                    "total": 12451.04,
                    "delivery_eta": "2 days"
                }
            ],
            "checkout_data": { "currency": "NGN" }
        }
    })
}

fn label_response() -> Value {
    // Documented sample (create shipment), trimmed.
    json!({
        "status": "success",
        "message": "Order successfully routed to Dellyman",
        "data": {
            "order_id": "SB-2CF48224272",
            "courier": { "name": "Dellyman", "email": "ops@dellyman.test", "phone": "010000038198" },
            "status": "pending",
            "payment": { "shipping_fee": 2785, "type": "wallet", "status": "completed", "currency": "NGN" },
            "tracking_url": "https://track.shipbubble.test/SB-2CF48224272",
            "date": "2026-10-31 01:23:19"
        }
    })
}

async fn mount_book(server: &MockServer, response: ResponseTemplate, times: u64) {
    Mock::given(method("POST"))
        .and(path("/v1/shipping/labels"))
        .respond_with(response)
        .expect(times)
        .mount(server)
        .await;
}

#[tokio::test]
async fn validate_address_returns_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shipping/address/validate"))
        .and(header(
            "authorization",
            format!("Bearer {API_KEY}").as_str(),
        ))
        .and(body_json(json!({
            "name": "Ada Obi",
            "email": "ada@example.test",
            "phone": "08031234567",
            "address": "12 Zambezi Crescent, AMAC, Federal Capital Territory, Nigeria"
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(validate_response("Federal Capital Territory", "FC")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let code = courier(&server).validate_address(&address()).await.unwrap();
    assert_eq!(code, 98_794_022);
}

#[tokio::test]
async fn validate_address_outside_fct_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shipping/address/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(validate_response("Niger", "NI")))
        .mount(&server)
        .await;
    let err = courier(&server)
        .validate_address(&address())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, CourierError::Rejected { code, .. } if code == "outside_service_area"),
        "{err:?}"
    );
}

#[tokio::test]
async fn quote_maps_couriers_and_amounts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shipping/fetch_rates"))
        .and(body_json(json!({
            "sender_address_code": 111,
            "reciever_address_code": 222,
            "pickup_date": "2026-11-04",
            "category_id": 3035980,
            "package_items": [{
                "name": "Blue kettle",
                "description": "Blue kettle",
                "unit_weight": "0.002",
                "unit_amount": "5000",
                "quantity": "2"
            }],
            "package_dimension": { "length": 20, "width": 15, "height": 10 }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(rates_response()))
        .expect(1)
        .mount(&server)
        .await;
    let pickup = Date::from_calendar_date(2026, Month::November, 4).unwrap();
    let quote = courier(&server)
        .quote(111, 222, pickup, &parcel())
        .await
        .unwrap();
    assert!(quote.request_token.starts_with("0f1e2d3c"));
    assert_eq!(quote.rates.len(), 2);
    let a = &quote.rates[0];
    assert_eq!(a.courier_id, "dellyman");
    assert_eq!(a.service_code, "dellyman");
    assert_eq!(a.amount_kobo, 278_500, "naira 2785 is 278,500 kobo");
    assert_eq!(a.charged_kobo, 290_050, "naira 2900.5 is 290,050 kobo");
    assert_eq!(a.delivery_eta.as_deref(), Some("Within 24 hours"));
    let b = &quote.rates[1];
    assert_eq!(b.courier_id, "1", "a numeric courier_id is stored as text");
    assert_eq!(b.amount_kobo, 1_245_104);
}

#[tokio::test]
async fn quote_with_unreadable_amount_is_an_error() {
    let server = MockServer::start().await;
    let mut body = rates_response();
    body["data"]["couriers"][0]["rate_card_amount"] = json!(2785.555);
    Mock::given(method("POST"))
        .and(path("/v1/shipping/fetch_rates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let pickup = Date::from_calendar_date(2026, Month::November, 4).unwrap();
    let err = courier(&server)
        .quote(1, 2, pickup, &parcel())
        .await
        .unwrap_err();
    assert!(matches!(err, CourierError::Inconclusive), "{err:?}");
}

#[test]
fn amounts_convert_exactly_and_never_round() {
    use AmountUnit::{Kobo, Naira};
    let ok = |v: Value, u| from_sb_amount(&v, u).ok();
    assert_eq!(ok(json!(2785), Naira), Some(278_500));
    assert_eq!(ok(json!(2785.5), Naira), Some(278_550));
    assert_eq!(ok(json!(12451.04), Naira), Some(1_245_104));
    assert_eq!(ok(json!(80124.2), Naira), Some(8_012_420));
    assert_eq!(ok(json!("2785"), Naira), Some(278_500));
    assert_eq!(ok(json!(0), Naira), Some(0));
    // More than two decimals, negative, non-numeric, overflow: all errors.
    assert_eq!(ok(json!(2785.555), Naira), None);
    assert_eq!(ok(json!(-1), Naira), None);
    assert_eq!(ok(json!("-1.5"), Naira), None);
    assert_eq!(ok(json!("abc"), Naira), None);
    assert_eq!(ok(json!("1e3"), Naira), None);
    assert_eq!(ok(json!(null), Naira), None);
    assert_eq!(ok(json!(true), Naira), None);
    assert_eq!(ok(json!(i64::MAX), Naira), None);
    // Kobo unit: whole integers only.
    assert_eq!(ok(json!(278_500), Kobo), Some(278_500));
    assert_eq!(ok(json!(278_500.5), Kobo), None);
    assert_eq!(ok(json!("100.5"), Kobo), None);

    let s = |k, u| to_sb_amount(k, u).ok();
    assert_eq!(s(500_000, Naira).as_deref(), Some("5000"));
    assert_eq!(s(278_550, Naira).as_deref(), Some("2785.5"));
    assert_eq!(s(278_505, Naira).as_deref(), Some("2785.05"));
    assert_eq!(s(1, Naira).as_deref(), Some("0.01"));
    assert_eq!(s(0, Naira).as_deref(), Some("0"));
    assert_eq!(s(278_550, Kobo).as_deref(), Some("278550"));
    assert_eq!(s(-1, Naira), None);
    assert_eq!(s(-1, Kobo), None);
}

#[tokio::test]
async fn book_success_is_parsed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shipping/labels"))
        .and(body_json(json!({
            "request_token": "tok",
            "service_code": "dellyman",
            "courier_id": "dellyman"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(label_response()))
        .expect(1)
        .mount(&server)
        .await;
    let b = courier(&server)
        .book("tok", "dellyman", "dellyman")
        .await
        .unwrap();
    assert_eq!(b.provider_order_id, "SB-2CF48224272");
    assert_eq!(b.courier_name, "Dellyman");
    assert_eq!(b.courier_phone.as_deref(), Some("010000038198"));
    assert_eq!(b.charged_kobo, Some(278_500));
    assert!(b.tracking_url.unwrap().contains("SB-2CF48224272"));
}

#[tokio::test]
async fn book_is_sent_once_and_timeout_is_timeout() {
    let server = MockServer::start().await;
    // The label "was created" but the answer comes after our timeout.
    mount_book(
        &server,
        ResponseTemplate::new(200)
            .set_body_json(label_response())
            .set_delay(Duration::from_millis(1500)),
        1,
    )
    .await;
    let mut cfg = config(&server.uri());
    cfg.timeout = Duration::from_millis(300);
    let c = ShipbubbleCourier::new(cfg).unwrap();
    let err = c.book("tok", "dellyman", "dellyman").await.unwrap_err();
    assert!(matches!(err, CourierError::Timeout), "{err:?}");
    // The mock's `expect(1)` is checked when the server drops: one request, no retry.
    tokio::time::sleep(Duration::from_millis(1700)).await;
}

#[tokio::test]
async fn book_connect_failure_is_unavailable() {
    // A port nothing listens on: connection refused, nothing was sent.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let c = ShipbubbleCourier::new(config(&format!("http://127.0.0.1:{port}"))).unwrap();
    let err = c.book("tok", "a", "b").await.unwrap_err();
    assert!(matches!(err, CourierError::Unavailable), "{err:?}");
}

#[tokio::test]
async fn book_4xx_is_rejected_and_409_is_timeout() {
    // Any 400 (a low wallet included) and other plain 4xx are Rejected, coded by status.
    for (status, code) in [(400, "http_400"), (401, "http_401"), (422, "http_422")] {
        let server = MockServer::start().await;
        mount_book(
            &server,
            ResponseTemplate::new(status).set_body_json(
                json!({"status": "failed", "message": "Insufficient wallet balance"}),
            ),
            1,
        )
        .await;
        let err = courier(&server).book("tok", "a", "b").await.unwrap_err();
        assert!(
            matches!(&err, CourierError::Rejected { code: c, .. } if c == code),
            "{status}: {err:?}"
        );
    }
    // Unsettled: the label may exist.
    for status in [408, 409, 425, 500, 502, 503] {
        let server = MockServer::start().await;
        mount_book(&server, ResponseTemplate::new(status), 1).await;
        let err = courier(&server).book("tok", "a", "b").await.unwrap_err();
        assert!(matches!(err, CourierError::Timeout), "{status}: {err:?}");
    }
    // Rate limited: not processed.
    let server = MockServer::start().await;
    mount_book(&server, ResponseTemplate::new(429), 1).await;
    let err = courier(&server).book("tok", "a", "b").await.unwrap_err();
    assert!(matches!(err, CourierError::Unavailable), "{err:?}");
    // A 2xx we cannot read may hide a label.
    let server = MockServer::start().await;
    mount_book(
        &server,
        ResponseTemplate::new(200).set_body_json(json!({"status": "success", "data": {}})),
        1,
    )
    .await;
    let err = courier(&server).book("tok", "a", "b").await.unwrap_err();
    assert!(matches!(err, CourierError::Timeout), "{err:?}");
}

#[tokio::test]
async fn cancel_maps_success_and_already_processed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shipping/labels/cancel/SB-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"status": "success", "message": "Shipment successfully cancelled"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/shipping/labels/cancel/SB-2"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            json!({"status": "failed", "message": "Shipment label already processed"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let c = courier(&server);
    c.cancel("SB-1").await.unwrap();
    let err = c.cancel("SB-2").await.unwrap_err();
    assert!(
        matches!(&err, CourierError::Rejected { code, .. } if code == "http_400"),
        "{err:?}"
    );
}

#[tokio::test]
async fn status_reads_the_first_list_result() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/shipping/labels/list/SB-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "success",
            "data": { "results": [{
                "order_id": "SB-1",
                "status": "in_transit",
                "courier": { "name": "Dellyman", "phone": 8099999999u64, "tracking_code": "T1" },
                "tracking_url": "https://track.shipbubble.test/SB-1"
            }]}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/shipping/labels/list/SB-9"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"status": "success", "data": {"results": []}})),
        )
        .mount(&server)
        .await;
    let c = courier(&server);
    let u = c.status("SB-1").await.unwrap();
    assert_eq!(u.state, ShipmentState::InTransit);
    assert_eq!(u.provider_order_id, "SB-1");
    assert_eq!(u.courier_phone.as_deref(), Some("8099999999"));
    let err = c.status("SB-9").await.unwrap_err();
    assert!(
        matches!(&err, CourierError::Rejected { code, .. } if code == "not_found"),
        "{err:?}"
    );
}

fn status_event(event: &str, status: &str) -> Value {
    // Documented sample shape (webhooks page), trimmed. The per-event bodies are
    // assumed to match the label.created sample.
    json!({
        "event": event,
        "order_id": "SB-2CF48224272",
        "status": status,
        "courier": { "name": "Dellyman", "phone": "010000038198", "tracking_code": "T1" },
        "tracking_url": "https://track.shipbubble.test/SB-2CF48224272",
        "date": "2022-09-11T12:03:46.000Z"
    })
}

#[test]
fn webhook_bad_signature_is_invalid() {
    let server_less = ShipbubbleCourier::new(config("http://127.0.0.1:1")).unwrap();
    let (mut headers, raw) = signed(&status_event("shipment.status.changed", "picked_up"));
    // Tampered body.
    let mut tampered = raw.clone();
    tampered.push(b' ');
    for (h, body) in [(headers.clone(), tampered), (HeaderMap::new(), raw.clone())] {
        let err = server_less.parse_webhook(&h, &body).unwrap_err();
        assert!(matches!(err, CourierError::InvalidSignature), "{err:?}");
    }
    // Malformed headers: not hex, odd length, empty, wrong length.
    for bad in ["zz", "abc", "", "00ff", "not hex at all"] {
        headers.insert(SIGNATURE_HEADER, HeaderValue::from_str(bad).unwrap());
        let err = server_less.parse_webhook(&headers, &raw).unwrap_err();
        assert!(
            matches!(err, CourierError::InvalidSignature),
            "{bad}: {err:?}"
        );
    }
    // Signed with another key.
    let mut mac = Hmac::<Sha512>::new_from_slice(b"another-key").unwrap();
    mac.update(&raw);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    headers.insert(SIGNATURE_HEADER, HeaderValue::from_str(&hex).unwrap());
    let err = server_less.parse_webhook(&headers, &raw).unwrap_err();
    assert!(matches!(err, CourierError::InvalidSignature), "{err:?}");
}

#[test]
fn webhook_status_changed_maps_state() {
    let c = ShipbubbleCourier::new(config("http://127.0.0.1:1")).unwrap();
    for (raw, want) in [
        ("pending", ShipmentState::Pending),
        ("confirmed", ShipmentState::Confirmed),
        ("picked_up", ShipmentState::PickedUp),
        ("in_transit", ShipmentState::InTransit),
        ("completed", ShipmentState::Completed),
        ("cancelled", ShipmentState::Cancelled),
    ] {
        let (h, body) = signed(&status_event("shipment.status.changed", raw));
        let u = c.parse_webhook(&h, &body).unwrap().expect("an update");
        assert_eq!(u.state, want, "{raw}");
        assert_eq!(u.provider_order_id, "SB-2CF48224272");
        assert_eq!(u.courier_phone.as_deref(), Some("010000038198"));
        assert!(u.occurred_at.is_some());
    }
    // shipment.cancelled, even without a status field.
    let mut cancelled = status_event("shipment.cancelled", "cancelled");
    cancelled.as_object_mut().unwrap().remove("status");
    let (h, body) = signed(&cancelled);
    let u = c.parse_webhook(&h, &body).unwrap().unwrap();
    assert_eq!(u.state, ShipmentState::Cancelled);
    // A minimal payload: only event, order_id and status; numeric order id.
    let (h, body) =
        signed(&json!({"event": "shipment.status.changed", "order_id": 77, "status": "completed"}));
    let u = c.parse_webhook(&h, &body).unwrap().unwrap();
    assert_eq!(
        (u.provider_order_id.as_str(), u.courier_phone),
        ("77", None)
    );
}

#[test]
fn webhook_unknown_event_is_ignored() {
    let c = ShipbubbleCourier::new(config("http://127.0.0.1:1")).unwrap();
    let ignored = [
        status_event("shipment.label.created", "pending"),
        status_event("shipment.sla.updated", "in_transit"),
        status_event("shipment.cod.remitted", "completed"),
        status_event("shipment.something.new", "completed"),
        json!({"event": "wallet.auto_topup.succeeded", "amount": 1000, "currency": "NGN", "ref_code": "r1"}),
        json!({"event": "wallet.auto_topup.failed", "reason": "charge_failed", "failure_count": 2}),
        json!({"event": "wallet.threshold.alert"}),
        // Unknown status string and missing order id: ignored, not an error.
        status_event("shipment.status.changed", "returned"),
        json!({"event": "shipment.status.changed", "status": "completed"}),
    ];
    for payload in ignored {
        let (h, body) = signed(&payload);
        assert_eq!(c.parse_webhook(&h, &body).unwrap(), None, "{payload}");
    }
    // A signed body that is not JSON cannot settle anything.
    let raw = b"not json".to_vec();
    let mut h = HeaderMap::new();
    h.insert(
        SIGNATURE_HEADER,
        HeaderValue::from_str(&sign(&raw)).unwrap(),
    );
    let err = c.parse_webhook(&h, &raw).unwrap_err();
    assert!(matches!(err, CourierError::Inconclusive), "{err:?}");
}

#[test]
fn debug_never_prints_key() {
    let c = ShipbubbleCourier::new(config("http://127.0.0.1:1")).unwrap();
    let cfg = config("http://127.0.0.1:1");
    let out = format!("{c:?} {c:#?} {cfg:?} {:?}", cfg.api_key);
    assert!(!out.contains(API_KEY), "{out}");
    assert!(!out.contains("9f8e7d6c"), "{out}");
}

#[tokio::test]
#[tracing_test::traced_test]
async fn errors_log_class_and_status_never_bodies() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/shipping/address/validate"))
        .respond_with(ResponseTemplate::new(422).set_body_json(
            json!({"status": "failed", "errors": ["bad address 12 Zambezi Crescent 08031234567"]}),
        ))
        .mount(&server)
        .await;
    let _ = courier(&server).validate_address(&address()).await;
    assert!(logs_contain("shipbubble"));
    assert!(logs_contain("validate_address"));
    for secret in [
        "Zambezi",
        "Ada Obi",
        "08031234567",
        API_KEY,
        "ada@example.test",
    ] {
        assert!(!logs_contain(secret), "{secret} was logged");
    }
}

#[test]
fn config_requires_sb_prod_and_unit_in_production() {
    const KEY_A: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
    const KEY_B: &str = "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=";
    let load = |pairs: &[(&str, &str)]| {
        let mut env: Vec<(String, String)> = [
            ("DATABASE_URL", "postgres://x"),
            ("FIELD_KEY", KEY_A),
            ("INDEX_KEY", KEY_B),
            (
                "PUBLIC_REDIRECT_URL",
                "https://shop.example/checkout/return",
            ),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        env.extend(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        Config::from_lookup(|name| {
            env.iter()
                .rev()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        })
    };
    let prod = [
        ("APP_ENV", "production"),
        ("MONO_SECRET_KEY", "live_sk_abcdef123456"),
        ("MONO_WEBHOOK_SECRET", "whsec"),
        ("MONO_DISBURSE_AMOUNT_UNIT", "kobo"),
        ("MONO_DISBURSE_SOURCE_ACCOUNT", "688a0b672f88771f77d05cf5"),
        ("MONO_PAYOUT_RECIPIENT_EMAIL", "payouts@platform.example"),
    ];
    let with = |extra: &[(&'static str, &'static str)]| {
        let mut v = prod.to_vec();
        v.extend_from_slice(extra);
        v
    };
    // No key: starts, courier delivery unavailable.
    assert!(load(&prod).unwrap().shipbubble.is_none());
    // A sandbox key, or a missing unit, is refused in production.
    let err = load(&with(&[
        ("SHIPBUBBLE_API_KEY", "sb_sandbox_abc"),
        ("SHIPBUBBLE_AMOUNT_UNIT", "naira"),
    ]))
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("sb_prod") && !err.contains("sb_sandbox_abc"),
        "{err}"
    );
    let err = load(&with(&[("SHIPBUBBLE_API_KEY", "sb_prod_abc")]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("SHIPBUBBLE_AMOUNT_UNIT"), "{err}");
    let ok = load(&with(&[
        ("SHIPBUBBLE_API_KEY", "sb_prod_abc"),
        ("SHIPBUBBLE_AMOUNT_UNIT", "kobo"),
    ]))
    .unwrap()
    .shipbubble
    .unwrap();
    assert!(ok.live && ok.amount_unit == AmountUnit::Kobo);
    // An sb_prod key outside production is refused; the unit defaults to naira there.
    assert!(load(&[("SHIPBUBBLE_API_KEY", "sb_prod_abc")]).is_err());
    let sandbox = load(&[("SHIPBUBBLE_API_KEY", "sb_sandbox_abc")])
        .unwrap()
        .shipbubble
        .unwrap();
    assert!(!sandbox.live && sandbox.amount_unit == AmountUnit::Naira);
    // Non-https base URL is refused.
    assert!(
        load(&[
            ("SHIPBUBBLE_API_KEY", "sb_sandbox_abc"),
            (
                "SHIPBUBBLE_BASE_URL",
                "http://api.shipbubble.com/v1/shipping"
            ),
        ])
        .is_err()
    );
}

/// Run by hand against the sandbox: `SHIPBUBBLE_SMOKE_API_KEY=sb_sandbox_... cargo test
/// --test shipbubble_adapter -- --ignored sandbox_smoke_test --nocapture`.
/// Confirms the amount unit (compare the printed kobo with the dashboard price) and the
/// FCT `state` values. Result: not yet run.
#[tokio::test]
#[ignore = "needs a Shipbubble sandbox key (SHIPBUBBLE_SMOKE_API_KEY)"]
async fn sandbox_smoke_test() {
    let Ok(key) = std::env::var("SHIPBUBBLE_SMOKE_API_KEY") else {
        panic!("set SHIPBUBBLE_SMOKE_API_KEY to a Shipbubble sb_sandbox key");
    };
    assert!(
        key.starts_with("sb_sandbox"),
        "the smoke test only runs with a sandbox key"
    );
    let base = std::env::var("SHIPBUBBLE_SMOKE_BASE_URL")
        .unwrap_or_else(|_| "https://api.shipbubble.com".into());
    let mut cfg = config(&base);
    cfg.base_url = format!("{base}/v1/shipping");
    cfg.api_key = Secret::new(&key);
    cfg.insecure_http_for_tests = false;
    let c = ShipbubbleCourier::new(cfg).unwrap();
    let code = c
        .validate_address(&ContactAddress {
            name: "Smoke Test".into(),
            email: Some("smoke@example.com".into()),
            phone: "08000000000".into(),
            address_line: "Plot 1 Ahmadu Bello Way, Garki".into(),
            area: AreaCouncil::Amac,
        })
        .await
        .expect("validate");
    println!("address_code: {code}");
    let pickup = (time::OffsetDateTime::now_utc() + time::Duration::days(1)).date();
    let quote = c.quote(code, code, pickup, &parcel()).await.expect("quote");
    for r in &quote.rates {
        println!(
            "{} {} rate_card {} kobo, total {} kobo",
            r.courier_id, r.service_code, r.amount_kobo, r.charged_kobo
        );
    }
}
