//! A few public pages the app stores require, served by the API itself so there is nothing
//! else to host. Today: `/account-deletion`, where a person who cannot open the app can
//! sign in and ask for their account to be deleted, or change their mind.
//!
//! The page has no inline script or style, so its content-security policy allows only this
//! origin; it sets no cookies and keeps the password in page memory only until it is used.
//! The only server data it shows is the support address from the app config.

use axum::{
    Router,
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::AppState;

const INDEX: &str = include_str!("../web/account-deletion/index.html");
const SCRIPT: &str = include_str!("../web/account-deletion/page.js");
const STYLE: &str = include_str!("../web/account-deletion/page.css");

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/account-deletion", get(page))
        .route("/account-deletion/page.js", get(script))
        .route("/account-deletion/page.css", get(style))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The page with the support sentence filled in (or left out).
pub fn render_page(support_email: Option<&str>) -> String {
    let support = match support_email {
        Some(e) => {
            let e = escape(e);
            format!("Questions, or cannot sign in? Write to <a href=\"mailto:{e}\">{e}</a>.")
        }
        None => String::new(),
    };
    INDEX.replace("{{SUPPORT}}", &support)
}

fn with_headers(content_type: &'static str, body: String, csp: bool) -> Response {
    let mut res = body.into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if csp {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; \
                 form-action 'none'; base-uri 'none'; frame-ancestors 'none'",
            ),
        );
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    } else {
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=300"),
        );
    }
    res
}

async fn page(State(state): State<AppState>) -> Response {
    with_headers(
        "text/html; charset=utf-8",
        render_page(state.app_config.support_email.as_deref()),
        true,
    )
}

async fn script() -> Response {
    with_headers("text/javascript; charset=utf-8", SCRIPT.to_string(), false)
}

async fn style() -> Response {
    with_headers("text/css; charset=utf-8", STYLE.to_string(), false)
}
