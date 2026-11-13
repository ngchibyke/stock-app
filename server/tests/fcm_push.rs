//! FcmPush against a local mock server (wiremock). No real network. The key
//! in `fixtures/fcm_test_key.pem` is a throwaway generated for these tests.

use std::time::Duration;

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde_json::{Value, json};
use server::notify::{
    fcm::{FcmConfig, FcmPush, Secret},
    push::{PushError, PushMessage, PushSender},
};
use tracing_test::traced_test;
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

const PROJECT: &str = "demo-project";
const EMAIL: &str = "push@demo-project.iam.gserviceaccount.com";
const DEVICE: &str = "device-token-AAAA1111";
const ACCESS: &str = "ya29.access-token-ZZZZ";
const SEND_PATH: &str = "/v1/projects/demo-project/messages:send";

/// A fixture PEM without its `#` header line.
fn pem(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .lines()
    .filter(|l| !l.starts_with('#'))
    .collect::<Vec<_>>()
    .join("\n")
}

fn config(server: &MockServer) -> FcmConfig {
    FcmConfig {
        project_id: PROJECT.into(),
        client_email: EMAIL.into(),
        private_key: Secret::new(&pem("fcm_test_key.pem")),
        token_uri: format!("{}/token", server.uri()),
        base_url: server.uri(),
        connect_timeout: Duration::from_millis(500),
        timeout: Duration::from_millis(400),
        insecure_http_for_tests: true,
    }
}

fn message(id: Uuid) -> PushMessage<'static> {
    PushMessage {
        token: DEVICE,
        title: "Delivered",
        body: "Your order was marked delivered.",
        kind: "order_delivered",
        notification_id: id,
    }
}

async fn token_endpoint(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": ACCESS, "expires_in": 3600, "token_type": "Bearer"
        })))
        .mount(server)
        .await;
}

/// A server whose token endpoint works and whose send endpoint answers `send`.
async fn serve(send: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    token_endpoint(&server).await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(send)
        .mount(&server)
        .await;
    server
}

fn ok() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "name": "projects/demo-project/messages/1" }))
}

async fn calls(server: &MockServer, to: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == to)
        .collect()
}

async fn outcome(send: ResponseTemplate) -> Result<(), PushError> {
    let server = serve(send).await;
    FcmPush::new(config(&server))
        .unwrap()
        .send(&message(Uuid::new_v4()))
        .await
}

#[tokio::test]
async fn mints_and_caches_the_token() {
    let server = serve(ok()).await;
    let cfg = config(&server);
    let push = FcmPush::new(cfg.clone()).unwrap();
    push.send(&message(Uuid::new_v4())).await.unwrap();
    push.send(&message(Uuid::new_v4())).await.unwrap();

    let minted = calls(&server, "/token").await;
    assert_eq!(minted.len(), 1, "the access token is cached");
    assert_eq!(calls(&server, SEND_PATH).await.len(), 2);

    let req = &minted[0];
    assert_eq!(
        req.headers.get("content-type").unwrap(),
        "application/x-www-form-urlencoded"
    );
    let form = reqwest::Url::parse(&format!(
        "http://x/?{}",
        String::from_utf8(req.body.clone()).unwrap()
    ))
    .unwrap();
    let field = |name: &str| {
        form.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
            .unwrap()
    };
    assert_eq!(form.query_pairs().count(), 2);
    assert_eq!(
        field("grant_type"),
        "urn:ietf:params:oauth:grant-type:jwt-bearer"
    );
    let jwt = field("assertion");
    assert_eq!(decode_header(&jwt).unwrap().alg, Algorithm::RS256);

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[cfg.token_uri.as_str()]);
    let key = DecodingKey::from_rsa_pem(pem("fcm_test_key.pub.pem").as_bytes()).unwrap();
    let claims = decode::<Value>(&jwt, &key, &validation).unwrap().claims;
    assert_eq!(claims["iss"], EMAIL);
    assert_eq!(
        claims["scope"],
        "https://www.googleapis.com/auth/firebase.messaging"
    );
    assert_eq!(claims["aud"], cfg.token_uri);
    let (iat, exp) = (
        claims["iat"].as_i64().unwrap(),
        claims["exp"].as_i64().unwrap(),
    );
    assert_eq!(exp - iat, 3600);
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    assert!((now - iat).abs() < 60, "iat is now");
    assert_eq!(claims.as_object().unwrap().len(), 5, "no other claims");
}

#[tokio::test]
async fn sends_the_exact_body() {
    let server = serve(ok()).await;
    let id = Uuid::new_v4();
    FcmPush::new(config(&server))
        .unwrap()
        .send(&message(id))
        .await
        .unwrap();
    let sent = calls(&server, SEND_PATH).await;
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].headers.get("authorization").unwrap(),
        &format!("Bearer {ACCESS}")
    );
    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "message": {
                "token": DEVICE,
                "notification": {
                    "title": "Delivered",
                    "body": "Your order was marked delivered.",
                },
                "data": { "kind": "order_delivered", "notification_id": id.to_string() },
            }
        })
    );
}

#[tokio::test]
async fn refreshes_once_on_401() {
    // The first send is refused; the token is minted again and the retry goes.
    let server = MockServer::start().await;
    token_endpoint(&server).await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(SEND_PATH))
        .respond_with(ok())
        .mount(&server)
        .await;
    let push = FcmPush::new(config(&server)).unwrap();
    push.send(&message(Uuid::new_v4())).await.unwrap();
    assert_eq!(calls(&server, "/token").await.len(), 2);
    assert_eq!(calls(&server, SEND_PATH).await.len(), 2);

    // A send that keeps answering 401 is tried twice, no more.
    let server = serve(ResponseTemplate::new(401)).await;
    let push = FcmPush::new(config(&server)).unwrap();
    assert_eq!(
        push.send(&message(Uuid::new_v4())).await,
        Err(PushError::Unavailable)
    );
    assert_eq!(calls(&server, SEND_PATH).await.len(), 2);
    assert_eq!(calls(&server, "/token").await.len(), 2);
}

fn fcm_error(code: u16, status: &str, error_code: &str) -> ResponseTemplate {
    ResponseTemplate::new(code).set_body_json(json!({
        "error": {
            "code": code,
            "message": "provider-text-do-not-log",
            "status": status,
            "details": [{
                "@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError",
                "errorCode": error_code,
            }],
        }
    }))
}

#[tokio::test]
async fn maps_errors() {
    assert_eq!(
        outcome(fcm_error(404, "NOT_FOUND", "UNREGISTERED")).await,
        Err(PushError::Unregistered)
    );
    // A 404 that does not name the token (a wrong project, say) must not
    // disable anyone's device.
    assert_eq!(
        outcome(ResponseTemplate::new(404).set_body_string("not found")).await,
        Err(PushError::Unavailable)
    );
    assert_eq!(
        outcome(fcm_error(400, "INVALID_ARGUMENT", "INVALID_ARGUMENT")).await,
        Err(PushError::Rejected)
    );
    for code in [429, 500, 503] {
        assert_eq!(
            outcome(ResponseTemplate::new(code)).await,
            Err(PushError::Unavailable),
            "{code}"
        );
    }
    assert_eq!(
        outcome(ok().set_delay(Duration::from_millis(900))).await,
        Err(PushError::Unavailable),
        "timeout"
    );
    assert_eq!(
        outcome(ResponseTemplate::new(200).set_body_string("x".repeat(64 * 1024 + 1))).await,
        Err(PushError::Unavailable),
        "body over 64 KiB"
    );
}

#[tokio::test]
async fn token_endpoint_failure_is_unavailable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let push = FcmPush::new(config(&server)).unwrap();
    assert_eq!(
        push.send(&message(Uuid::new_v4())).await,
        Err(PushError::Unavailable)
    );
    assert!(calls(&server, SEND_PATH).await.is_empty());
    // While the token endpoint is down, the next sends fail at once instead
    // of each waiting on it in turn.
    for _ in 0..3 {
        assert_eq!(
            push.send(&message(Uuid::new_v4())).await,
            Err(PushError::Unavailable)
        );
    }
    assert_eq!(calls(&server, "/token").await.len(), 1);
}

#[tokio::test]
#[traced_test]
async fn errors_and_debug_hide_token_and_key() {
    let server = serve(fcm_error(500, "INTERNAL", "INTERNAL")).await;
    let cfg = config(&server);
    let err = FcmPush::new(cfg.clone())
        .unwrap()
        .send(&message(Uuid::new_v4()))
        .await
        .unwrap_err();
    let server = serve(fcm_error(404, "NOT_FOUND", "UNREGISTERED")).await;
    let _ = FcmPush::new(config(&server))
        .unwrap()
        .send(&message(Uuid::new_v4()))
        .await;

    let debug = format!("{err:?} {:?} {cfg:?}", message(Uuid::new_v4()));
    for secret in [DEVICE, "PRIVATE KEY", "provider-text-do-not-log"] {
        assert!(!debug.contains(secret), "Debug shows {secret}: {debug}");
    }
    for secret in [DEVICE, ACCESS, "PRIVATE KEY", "provider-text-do-not-log"] {
        assert!(!logs_contain(secret), "logs show {secret}");
    }
    assert!(logs_contain("push failed"), "the failure is still logged");
}

fn service_account(token_uri: &str) -> String {
    let dir = std::env::temp_dir().join(format!("fcm-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("service-account.json");
    std::fs::write(
        &file,
        json!({
            "type": "service_account",
            "project_id": PROJECT,
            "private_key_id": "abc",
            "private_key": pem("fcm_test_key.pem"),
            "client_email": EMAIL,
            "token_uri": token_uri,
        })
        .to_string(),
    )
    .unwrap();
    file.to_str().unwrap().to_string()
}

#[test]
fn reads_a_service_account_file() {
    let cfg = FcmConfig::from_service_account_file(&service_account(
        "https://oauth2.googleapis.com/token",
    ))
    .unwrap();
    assert_eq!(cfg.project_id, PROJECT);
    assert_eq!(cfg.client_email, EMAIL);
    assert_eq!(cfg.base_url, "https://fcm.googleapis.com");
    assert!(!cfg.insecure_http_for_tests);
    assert!(FcmPush::new(cfg).is_ok());
}

#[test]
fn token_uri_must_be_google() {
    let err = FcmConfig::from_service_account_file(&service_account("https://evil.example/token"))
        .unwrap_err();
    assert!(err.to_string().contains("token_uri"), "{err}");
}

#[test]
fn a_bad_key_or_file_is_refused_without_echoing_it() {
    let err = FcmConfig::from_service_account_file("/no/such/file.json").unwrap_err();
    assert!(!format!("{err:#}").contains("PRIVATE"), "{err:#}");
    let server_less = FcmConfig {
        project_id: PROJECT.into(),
        client_email: EMAIL.into(),
        private_key: Secret::new("not a key"),
        token_uri: "https://oauth2.googleapis.com/token".into(),
        base_url: "https://fcm.googleapis.com".into(),
        connect_timeout: Duration::from_secs(1),
        timeout: Duration::from_secs(1),
        insecure_http_for_tests: false,
    };
    let err = FcmPush::new(server_less).err().unwrap();
    assert!(!format!("{err:#}").contains("not a key"), "{err:#}");
}
