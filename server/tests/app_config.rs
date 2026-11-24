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
            "assets": null,
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
            "assets": null,
        })
    );
}

const ASSETS_URL: &str = "https://cdn.shop.example/market/";

#[test]
fn assets_are_null_when_unset() {
    assert_eq!(parse(&[]).unwrap().assets, None);
    assert_eq!(parse(&[]).unwrap().view()["assets"], json!(null));
    // Blank counts as unset.
    let blank = parse(&[
        ("MARKET_ASSETS_VERSION", "  "),
        ("MARKET_ASSETS_BASE_URL", ""),
    ])
    .unwrap();
    assert_eq!(blank.assets, None);
    // One blank and one set is the same as only one set.
    assert!(
        parse(&[
            ("MARKET_ASSETS_VERSION", "v1"),
            ("MARKET_ASSETS_BASE_URL", " "),
        ])
        .is_err()
    );
}

#[sqlx::test]
async fn assets_are_served_when_both_are_set(pool: PgPool) {
    let config = parse(&[
        ("MARKET_ASSETS_VERSION", " 2026-11.a_b "),
        ("MARKET_ASSETS_BASE_URL", ASSETS_URL),
    ])
    .unwrap();
    let assets = config.assets.clone().unwrap();
    assert_eq!(assets.version, "2026-11.a_b");
    assert_eq!(assets.base_url, ASSETS_URL);

    let mut app = TestApp::new(pool).await;
    app.use_app_config(config);
    let res = app.request(Method::GET, "/app/config", None, None).await;
    assert_eq!(res.status, 200, "body: {}", res.json);
    assert_eq!(
        res.json["assets"],
        json!({ "version": "2026-11.a_b", "base_url": ASSETS_URL })
    );
    assert_eq!(
        res.headers.get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=300"
    );
}

#[test]
fn only_one_of_the_two_is_refused() {
    for pairs in [
        [("MARKET_ASSETS_VERSION", "v1")],
        [("MARKET_ASSETS_BASE_URL", ASSETS_URL)],
    ] {
        let err = parse(&pairs).unwrap_err().to_string();
        assert!(
            err.contains("MARKET_ASSETS_VERSION") && err.contains("MARKET_ASSETS_BASE_URL"),
            "{err}"
        );
    }
}

#[test]
fn a_non_https_base_url_is_refused() {
    for bad in [
        "http://cdn.shop.example/market/",
        "https://",
        "cdn.shop.example/market/",
        "https://cdn.shop.example/mar ket/",
    ] {
        let err = parse(&[
            ("MARKET_ASSETS_VERSION", "v1"),
            ("MARKET_ASSETS_BASE_URL", bad),
        ])
        .unwrap_err()
        .to_string();
        assert!(err.contains("MARKET_ASSETS_BASE_URL"), "{bad}: {err}");
    }
}

#[test]
fn a_bad_version_is_refused() {
    let long = "a".repeat(41);
    for bad in ["", "has space", "a/b", "..", "a..b", long.as_str()] {
        let result = parse(&[
            ("MARKET_ASSETS_VERSION", bad),
            ("MARKET_ASSETS_BASE_URL", ASSETS_URL),
        ]);
        // An empty value is blank, so it is the "only one set" error; both name the variable.
        let err = result.unwrap_err().to_string();
        assert!(err.contains("MARKET_ASSETS_VERSION"), "{bad:?}: {err}");
    }
    // The longest allowed version passes.
    let max = "a".repeat(40);
    assert!(
        parse(&[
            ("MARKET_ASSETS_VERSION", max.as_str()),
            ("MARKET_ASSETS_BASE_URL", ASSETS_URL),
        ])
        .is_ok()
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
