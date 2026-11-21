mod common;

use std::collections::HashMap;

use axum::http::{Method, header};
use common::TestApp;
use serde_json::json;
use server::app_config::{AppClientConfig, MAX_MESSAGE_CHARS};
use sqlx::PgPool;

fn parse(pairs: &[(&str, &str)]) -> anyhow::Result<AppClientConfig> {
    let env: HashMap<&str, &str> = pairs.iter().copied().collect();
    AppClientConfig::from_lookup(&|k: &str| env.get(k).map(|v| v.to_string()))
}

fn full() -> AppClientConfig {
    parse(&[
        ("APP_MIN_VERSION_ANDROID", "1.4.0"),
        ("APP_MIN_VERSION_IOS", " 1.5.2 "),
        ("APP_TERMS_URL", "https://shop.example/terms"),
        ("APP_PRIVACY_URL", "https://shop.example/privacy"),
        ("APP_SUPPORT_EMAIL", "help@shop.example"),
        ("APP_MAINTENANCE_MESSAGE", "Back at 6pm."),
    ])
    .unwrap()
}

#[sqlx::test]
async fn config_route_reads_settings(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.use_app_config(full());
    let res = app.request(Method::GET, "/app/config", None, None).await;
    assert_eq!(res.status, 200, "body: {}", res.json);
    assert_eq!(
        res.json,
        json!({
            "min_version": { "android": "1.4.0", "ios": "1.5.2" },
            "terms_url": "https://shop.example/terms",
            "privacy_url": "https://shop.example/privacy",
            "support_email": "help@shop.example",
            "maintenance": { "active": true, "message": "Back at 6pm." },
        })
    );
}

#[test]
fn bad_version_or_http_link_refuses_to_start() {
    for (name, bad) in [
        ("APP_MIN_VERSION_ANDROID", "1.4"),
        ("APP_MIN_VERSION_ANDROID", "v1.4.0"),
        ("APP_MIN_VERSION_IOS", "1.4.0-beta"),
        ("APP_MIN_VERSION_IOS", "01.4.0"),
        ("APP_MIN_VERSION_IOS", "1..0"),
        ("APP_MIN_VERSION_IOS", "1.4.0.1"),
        ("APP_TERMS_URL", "http://shop.example/terms"),
        ("APP_TERMS_URL", "shop.example/terms"),
        ("APP_TERMS_URL", "https://"),
        ("APP_PRIVACY_URL", "https://shop example/privacy"),
        ("APP_PRIVACY_URL", "javascript:alert(1)"),
        ("APP_SUPPORT_EMAIL", "not-an-email"),
        ("APP_SUPPORT_EMAIL", "a@b"),
        ("APP_SUPPORT_EMAIL", "a b@shop.example"),
    ] {
        let err = parse(&[(name, bad)]).unwrap_err().to_string();
        assert!(err.contains(name), "{name}={bad}: {err}");
        assert!(!err.contains(bad), "the value must not be echoed: {err}");
    }
    let long = "x".repeat(MAX_MESSAGE_CHARS + 1);
    assert!(parse(&[("APP_MAINTENANCE_MESSAGE", long.as_str())]).is_err());
    assert!(parse(&[("APP_MAINTENANCE_MESSAGE", "two\nlines")]).is_err());
}

#[sqlx::test]
async fn defaults_are_null_not_empty(pool: PgPool) {
    // Unset and blank settings both mean null; maintenance is off.
    let blank = parse(&[
        ("APP_MIN_VERSION_ANDROID", "  "),
        ("APP_TERMS_URL", ""),
        ("APP_MAINTENANCE_MESSAGE", " "),
    ])
    .unwrap();
    assert_eq!(blank, parse(&[]).unwrap());

    let mut app = TestApp::new(pool).await;
    app.use_app_config(blank);
    let res = app.request(Method::GET, "/app/config", None, None).await;
    assert_eq!(res.status, 200);
    assert_eq!(
        res.json,
        json!({
            "min_version": { "android": null, "ios": null },
            "terms_url": null,
            "privacy_url": null,
            "support_email": null,
            "maintenance": { "active": false, "message": null },
        })
    );
}

#[sqlx::test]
async fn response_is_cacheable_and_has_no_user_data(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.use_app_config(full());
    // Public: no sign-in needed, and a signed-in caller gets the very same answer.
    let (_, token) = app.sign_up_verified("08031234567").await;
    let anonymous = app.request(Method::GET, "/app/config", None, None).await;
    let signed_in = app
        .request(Method::GET, "/app/config", None, Some(&token))
        .await;
    assert_eq!(anonymous.status, 200);
    assert_eq!(anonymous.json, signed_in.json);
    assert_eq!(
        anonymous.headers.get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=300"
    );
    assert!(anonymous.headers.get(header::SET_COOKIE).is_none());
    let text = anonymous.json.to_string();
    assert!(!text.contains("8031234567") && !text.contains("Ada"));
}

#[sqlx::test]
async fn config_route_rejects_other_methods(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = app
        .request(Method::POST, "/app/config", Some(json!({})), None)
        .await;
    assert_eq!(res.status, 405);
}
