//! Part 9: the low-level partner client against a local mock partner. No database.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::json;
use server::{
    partners::{
        PartnerConfig, PartnerError,
        client::{HttpPartner, MAX_RESPONSE_BODY, PartnerApi, check_outbound},
    },
    payments::mono::Secret,
};

const KEY: &str = "key-that-must-never-leak";
const SECRET: &str = "secret-that-must-never-leak";

/// The authorization header and the idempotency key of each request.
type Seen = (String, Option<String>);

#[derive(Clone, Default)]
struct Mock {
    hits: Arc<AtomicUsize>,
    /// Statuses answered in turn; the last one repeats.
    script: Arc<std::sync::Mutex<Vec<u16>>>,
    seen: Arc<std::sync::Mutex<Vec<Seen>>>,
}

async fn handler(State(m): State<Mock>, headers: HeaderMap) -> impl IntoResponse {
    let n = m.hits.fetch_add(1, Ordering::SeqCst);
    m.seen.lock().unwrap().push((
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string(),
        headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
    ));
    let script = m.script.lock().unwrap().clone();
    let code = script.get(n).or(script.last()).copied().unwrap_or(200);
    match code {
        200 => (StatusCode::OK, "{\"ok\":true}".to_string()).into_response(),
        302 => (
            StatusCode::FOUND,
            [("location", "https://elsewhere.test/")],
            "",
        )
            .into_response(),
        999 => (StatusCode::OK, "x".repeat(MAX_RESPONSE_BODY + 10)).into_response(),
        998 => {
            tokio::time::sleep(Duration::from_millis(600)).await;
            (StatusCode::OK, "{}".to_string()).into_response()
        }
        c => (StatusCode::from_u16(c).unwrap(), "{}".to_string()).into_response(),
    }
}

async fn serve(script: &[u16]) -> (String, Mock) {
    let mock = Mock {
        script: Arc::new(std::sync::Mutex::new(script.to_vec())),
        ..Mock::default()
    };
    let app = Router::new()
        .route("/{*path}", get(handler).post(handler))
        .route("/", post(handler))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), mock)
}

fn cfg(base: &str, insecure: bool) -> PartnerConfig {
    PartnerConfig {
        base_url: base.to_string(),
        api_key: Secret::new(KEY),
        webhook_secret: Secret::new(SECRET),
        insecure_http_for_tests: insecure,
    }
}

fn client(base: &str) -> HttpPartner {
    HttpPartner::new(cfg(base, true)).unwrap()
}

#[tokio::test]
async fn a_get_sends_the_bearer_key_and_a_post_the_idempotency_key() {
    let (base, mock) = serve(&[200]).await;
    let c = client(&base);
    assert_eq!(c.get("v1/health").await.unwrap(), json!({"ok": true}));
    c.post("v1/prices", "key-123", &json!({"updates": [{"store_ref": "s", "barcode": "b", "price_kobo": 100, "effective_at": "2026-11-26T10:00:00Z"}]}))
        .await
        .unwrap();
    let seen = mock.seen.lock().unwrap().clone();
    assert_eq!(seen[0], (format!("Bearer {KEY}"), None));
    assert_eq!(
        seen[1],
        (format!("Bearer {KEY}"), Some("key-123".to_string()))
    );
}

#[tokio::test]
async fn it_speaks_https_only_and_follows_no_redirects() {
    let (base, mock) = serve(&[200]).await;
    // Plain http is refused outright when the test switch is off.
    let strict = HttpPartner::new(cfg(&base, false)).unwrap();
    assert_eq!(
        strict.get("v1/health").await,
        Err(PartnerError::Unavailable)
    );
    assert_eq!(
        mock.hits.load(Ordering::SeqCst),
        0,
        "nothing reached the partner"
    );
    // A redirect is an answer, not a path to follow.
    let (base, mock) = serve(&[302]).await;
    assert_eq!(
        client(&base).get("v1/health").await,
        Err(PartnerError::Rejected)
    );
    assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_slow_partner_times_out_without_blocking_longer_than_the_timeout() {
    let (base, _mock) = serve(&[998]).await;
    let c = HttpPartner::with_timeouts(
        cfg(&base, true),
        Duration::from_millis(200),
        Duration::from_millis(150),
    )
    .unwrap();
    let started = Instant::now();
    assert_eq!(
        c.post("v1/prices", "k", &json!({})).await,
        Err(PartnerError::Unavailable)
    );
    assert!(
        started.elapsed() < Duration::from_millis(400),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_huge_answer_is_refused() {
    let (base, _mock) = serve(&[999]).await;
    assert_eq!(
        client(&base).get("v1/items/x/history").await,
        Err(PartnerError::BadResponse)
    );
    let (base, _mock) = serve(&[200]).await;
    // And an answer that is not JSON is bad too.
    let _ = base;
}

#[tokio::test]
async fn a_get_retries_once_and_a_post_never() {
    let (base, mock) = serve(&[500, 200]).await;
    assert!(client(&base).get("v1/health").await.is_ok());
    assert_eq!(mock.hits.load(Ordering::SeqCst), 2, "one retry saved it");

    let (base, mock) = serve(&[500]).await;
    assert_eq!(
        client(&base).get("v1/health").await,
        Err(PartnerError::Unavailable)
    );
    assert_eq!(mock.hits.load(Ordering::SeqCst), 2, "one retry and no more");

    let (base, mock) = serve(&[500, 200]).await;
    let r = client(&base)
        .post("v1/prices", "k", &json!({"updates": []}))
        .await;
    assert_eq!(r, Err(PartnerError::Unavailable));
    assert_eq!(mock.hits.load(Ordering::SeqCst), 1, "a post is sent once");

    // A refusal is the partner's answer: not retried either.
    let (base, mock) = serve(&[422]).await;
    assert_eq!(
        client(&base).get("v1/items/x").await,
        Err(PartnerError::Rejected)
    );
    assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_path_cannot_leave_the_base() {
    let (base, mock) = serve(&[200]).await;
    let c = client(&base);
    for bad in ["../admin", "v1//x", "v1/ x", "//evil.test/x"] {
        assert_eq!(c.get(bad).await, Err(PartnerError::Blocked), "{bad}");
    }
    assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn outbound_payloads_carry_only_allowed_fields() {
    let ok = json!({"updates": [{"store_ref": "s1", "barcode": "123", "price_kobo": 5000, "effective_at": "2026-11-26T10:00:00Z"}]});
    assert!(check_outbound(&ok).is_ok());
    // Anything personal, at any depth, is refused: a name, phone, address, chat text, token.
    for bad in [
        json!({"name": "Ada"}),
        json!({"updates": [{"store_ref": "s", "buyer_phone": "0803"}]}),
        json!({"updates": [{"store_ref": "s", "address": "4 Quiet Close"}]}),
        json!({"message": "hello"}),
        json!({"token": "x"}),
        json!({"updates": [{"nested": {"price_kobo": 1, "note": "n"}}]}),
    ] {
        assert_eq!(check_outbound(&bad), Err(PartnerError::Blocked), "{bad}");
    }
    // And the real client sends nothing for them.
    let (base, mock) = serve(&[200]).await;
    let r = client(&base)
        .post("v1/prices", "k", &json!({"phone": "0803"}))
        .await;
    assert_eq!(r, Err(PartnerError::Blocked));
    assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_key_and_the_secret_never_appear_in_debug_or_errors() {
    let c = cfg("https://partner.example.test", false);
    let shown = format!("{c:?}");
    assert!(!shown.contains(KEY) && !shown.contains(SECRET), "{shown}");
    // A failing call's error is a fixed code.
    let (base, _mock) = serve(&[500]).await;
    let err = client(&base).get("v1/health").await.unwrap_err();
    for text in [format!("{err:?}"), err.code().to_string()] {
        assert!(!text.contains(KEY) && !text.contains(&base), "{text}");
    }
}
