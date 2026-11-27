//! The public account-deletion page the stores require.

mod common;

use axum::http::Method;
use common::TestApp;
use server::{app_config::AppClientConfig, web::render_page};
use sqlx::PgPool;

async fn get(app: &TestApp, path: &str) -> (u16, axum::http::HeaderMap, String) {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let router = server::build_app(app.state.clone());
    let res = router
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[sqlx::test]
async fn the_page_explains_deletion_and_offers_sign_in_with_no_inline_code(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (status, h, body) = get(&app, "/account-deletion").await;
    assert_eq!(status, 200);
    assert_eq!(h["content-type"], "text/html; charset=utf-8");
    assert_eq!(h["cache-control"], "no-store");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["referrer-policy"], "no-referrer");
    let csp = h["content-security-policy"].to_str().unwrap();
    for part in [
        "default-src 'none'",
        "script-src 'self'",
        "connect-src 'self'",
        "frame-ancestors 'none'",
        "form-action 'none'",
    ] {
        assert!(csp.contains(part), "{csp}");
    }
    assert!(
        !csp.contains("unsafe-inline") && !csp.contains("unsafe-eval"),
        "{csp}"
    );
    for text in [
        "Delete your account",
        "30 days",
        "Keep my account",
        "Phone number",
        "Password",
        "Privacy and data",
    ] {
        assert!(body.contains(text), "missing {text}");
    }
    // Nothing inline: one external script, no handlers, no style attributes, no leftover slot.
    assert_eq!(body.matches("<script").count(), 1);
    assert!(body.contains("<script src=\"/account-deletion/page.js\">"));
    for bad in [
        "onclick=",
        "onsubmit=",
        "javascript:",
        "style=\"",
        "{{SUPPORT}}",
    ] {
        assert!(!body.contains(bad), "{bad}");
    }
    // The form fields have labels.
    for id in ["phone", "password"] {
        assert!(body.contains(&format!("for=\"{id}\"")) && body.contains(&format!("id=\"{id}\"")));
    }
}

#[sqlx::test]
async fn the_script_and_styles_are_served_and_keep_nothing_in_the_browser(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (status, h, js) = get(&app, "/account-deletion/page.js").await;
    assert_eq!(status, 200);
    assert_eq!(h["content-type"], "text/javascript; charset=utf-8");
    assert_eq!(h["x-content-type-options"], "nosniff");
    // The password and the token live in page memory only.
    for bad in [
        "localStorage",
        "sessionStorage",
        "document.cookie",
        "eval(",
        "innerHTML",
        "document.write",
    ] {
        assert!(!js.contains(bad), "{bad}");
    }
    assert!(js.contains("credentials: \"omit\""));
    // Every call goes to this server's own paths.
    for path in ["/auth/sign-in", "/me/deletion", "/auth/sign-out"] {
        assert!(js.contains(path), "{path}");
    }
    assert!(!js.contains("http://") && !js.contains("https://"));
    let (status, h, css) = get(&app, "/account-deletion/page.css").await;
    assert_eq!(status, 200);
    assert_eq!(h["content-type"], "text/css; charset=utf-8");
    assert!(css.contains("focus-visible"));
    // Anything else under it is not found, in the usual envelope.
    let (status, _, _) = get(&app, "/account-deletion/other").await;
    assert_eq!(status, 404);
}

#[sqlx::test]
async fn the_support_address_is_shown_when_there_is_one_and_always_escaped(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let (_, _, body) = get(&app, "/account-deletion").await;
    assert!(!body.contains("mailto:"), "no address configured, no line");
    app.use_app_config(
        AppClientConfig::from_lookup(&|n| {
            (n == "APP_SUPPORT_EMAIL").then(|| "help@shop.example".to_string())
        })
        .unwrap(),
    );
    let (_, _, body) = get(&app, "/account-deletion").await;
    assert!(body.contains("<a href=\"mailto:help@shop.example\">help@shop.example</a>"));
    // Whatever the address holds cannot break out of the page.
    let tricky = render_page(Some("a\"><script>x</script>@e.ng"));
    assert!(!tricky.contains("<script>x"), "{tricky}");
    assert!(tricky.contains("&lt;script&gt;x"));
}
