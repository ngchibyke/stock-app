//! ClaudeTone against a local mock server (wiremock). No real network.

use std::time::Duration;

use serde_json::{Value, json};
use server::moderation::{
    claude::{ClaudeConfig, ClaudeTone, SYSTEM_PROMPT, Secret, verdict_schema},
    filter::{Category, Severity},
    tone::{ToneChecker, ToneError},
};
use tracing_test::traced_test;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

const KEY: &str = "sk-ant-test-0123456789";
const TEXT: &str = "abeg this your price too high o";

fn tone_config(base_url: &str) -> ClaudeConfig {
    ClaudeConfig {
        api_key: Secret::new(KEY),
        base_url: base_url.into(),
        model: "claude-haiku-5-5".into(),
        connect_timeout: Duration::from_millis(500),
        timeout: Duration::from_millis(300),
        insecure_http_for_tests: true,
    }
}

fn tone(server: &MockServer) -> ClaudeTone {
    ClaudeTone::new(tone_config(&server.uri())).unwrap()
}

fn answer(text: Value) -> Value {
    json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "stop_reason": "end_turn",
        "content": [{ "type": "text", "text": text.to_string() }],
    })
}

async fn serve(template: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(template)
        .mount(&server)
        .await;
    server
}

async fn serve_json(body: Value) -> MockServer {
    serve(ResponseTemplate::new(200).set_body_json(body)).await
}

#[tokio::test]
async fn sends_only_the_text() {
    let server = serve_json(answer(
        json!({"severity": "clean", "category": "none", "suggestion": ""}),
    ))
    .await;
    tone(&server).check(TEXT).await.unwrap();

    let requests: Vec<Request> = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.headers.get("x-api-key").unwrap(), KEY);
    assert_eq!(req.headers.get("anthropic-version").unwrap(), "2026-10-21");
    let body: Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(
        body,
        json!({
            "model": "claude-haiku-5-5",
            "max_tokens": 4096,
            "system": SYSTEM_PROMPT,
            "messages": [{ "role": "user", "content": TEXT }],
            "output_config": {
                "effort": "low",
                "format": { "type": "json_schema", "schema": verdict_schema() },
            },
        })
    );
}

#[tokio::test]
async fn parses_a_verdict() {
    let clean = serve_json(answer(
        json!({"severity": "clean", "category": "none", "suggestion": ""}),
    ))
    .await;
    let v = tone(&clean).check(TEXT).await.unwrap();
    assert_eq!(v.severity, None);
    assert_eq!(v.category, None);
    assert_eq!(v.suggestion, None);

    let mild = serve_json(answer(
        json!({"severity": "mild", "category": "rude", "suggestion": "Please reconsider."}),
    ))
    .await;
    let v = tone(&mild).check(TEXT).await.unwrap();
    assert_eq!(v.severity, Some(Severity::Mild));
    assert_eq!(v.category, Some(Category::Rude));
    assert_eq!(v.suggestion.as_deref(), Some("Please reconsider."));

    let serious = serve_json(answer(
        json!({"severity": "serious", "category": "off_platform", "suggestion": ""}),
    ))
    .await;
    let v = tone(&serious).check(TEXT).await.unwrap();
    assert_eq!(v.severity, Some(Severity::Serious));
    assert_eq!(v.category, Some(Category::OffPlatform));
}

#[tokio::test]
async fn skips_thinking_blocks() {
    let server = serve_json(json!({
        "stop_reason": "end_turn",
        "content": [
            { "type": "thinking", "thinking": "", "signature": "x" },
            { "type": "text", "text": json!({
                "severity": "mild", "category": "rude", "suggestion": "Kindly check."
            }).to_string() },
        ],
    }))
    .await;
    let v = tone(&server).check(TEXT).await.unwrap();
    assert_eq!(v.suggestion.as_deref(), Some("Kindly check."));
}

#[tokio::test]
async fn refusal_is_refused() {
    let server = serve_json(json!({
        "stop_reason": "refusal",
        "stop_details": { "type": "refusal", "category": "general_harms" },
        "content": [],
    }))
    .await;
    assert_eq!(tone(&server).check(TEXT).await, Err(ToneError::Refused));
}

#[tokio::test]
async fn failures_are_unavailable() {
    let cases = vec![
        ResponseTemplate::new(429),
        ResponseTemplate::new(500),
        ResponseTemplate::new(529),
        ResponseTemplate::new(400).set_body_string("nope"),
        ResponseTemplate::new(200).set_delay(Duration::from_secs(2)),
        ResponseTemplate::new(200).set_body_json(json!({
            "stop_reason": "max_tokens",
            "content": [{ "type": "text", "text": "{\"severity\":" }],
        })),
        ResponseTemplate::new(200).set_body_json(json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "not json" }],
        })),
        ResponseTemplate::new(200).set_body_json(answer(
            json!({"severity": "furious", "category": "none", "suggestion": ""}),
        )),
        ResponseTemplate::new(200).set_body_json(answer(
            json!({"severity": "mild", "category": "new", "suggestion": ""}),
        )),
        ResponseTemplate::new(200).set_body_json(json!({
            "stop_reason": "end_turn",
            "content": [],
        })),
        ResponseTemplate::new(200).set_body_string("x".repeat(70 * 1024)),
    ];
    for (i, case) in cases.into_iter().enumerate() {
        let server = serve(case).await;
        assert_eq!(
            tone(&server).check(TEXT).await,
            Err(ToneError::Unavailable),
            "case {i}"
        );
    }
}

#[tokio::test]
async fn connection_failure_is_unavailable() {
    let server = MockServer::start().await;
    let checker = tone(&server);
    drop(server);
    assert_eq!(checker.check(TEXT).await, Err(ToneError::Unavailable));
}

#[tokio::test]
#[traced_test]
async fn errors_and_debug_hide_text() {
    let suggestion = "a very private suggestion";
    let ok = serve_json(answer(
        json!({"severity": "mild", "category": "rude", "suggestion": suggestion}),
    ))
    .await;
    let v = tone(&ok).check(TEXT).await.unwrap();
    let debug = format!("{v:?}");
    assert!(!debug.contains(suggestion), "{debug}");

    let failing = serve(ResponseTemplate::new(500).set_body_string(TEXT)).await;
    let _ = tone(&failing).check(TEXT).await;
    let refused = serve_json(json!({"stop_reason": "refusal", "content": []})).await;
    let _ = tone(&refused).check(TEXT).await;

    assert!(logs_contain("tone check failed"));
    assert!(!logs_contain(TEXT));
    assert!(!logs_contain(suggestion));
    assert!(!logs_contain(KEY));
    let config = format!("{:?}", tone_config("https://x"));
    assert!(!config.contains(KEY), "{config}");
}
