//! `app/openapi.yaml` stays true to the router (plan 7, S4).
//!
//! axum cannot list its routes, so each check asks the router itself:
//! - a path exists when a method nobody implements (`TRACE`) answers 405, not 404;
//! - a method exists when it does not answer 405.
//!
//! The other direction is a list of the routes the phone app uses, written out
//! here: a route missing from the file fails the test.

mod common;

use std::collections::BTreeSet;

use axum::http::Method;
use common::TestApp;
use sqlx::PgPool;

const SPEC: &str = include_str!("../../app/openapi.yaml");

/// Every route the phone app (buyer or seller) calls. Admin and webhook routes are not the app's.
const APP_ROUTES: &[(&str, &str)] = &[
    ("GET", "/config"),
    ("POST", "/auth/sign-up"),
    ("POST", "/auth/verify-phone"),
    ("POST", "/auth/password-reset"),
    ("POST", "/auth/password-reset/confirm"),
    ("POST", "/auth/sign-in"),
    ("POST", "/auth/sign-out"),
    ("GET", "/me"),
    ("GET", "/me/addresses"),
    ("POST", "/me/addresses"),
    ("DELETE", "/me/addresses/{id}"),
    ("GET", "/me/notifications"),
    ("POST", "/me/notifications/read"),
    ("PUT", "/me/push-device"),
    ("DELETE", "/me/push-device"),
    ("POST", "/uploads"),
    ("GET", "/categories"),
    ("GET", "/products"),
    ("GET", "/products/{id}"),
    ("PATCH", "/products/{id}"),
    ("DELETE", "/products/{id}"),
    ("PUT", "/products/{id}/variants"),
    ("POST", "/stores"),
    ("GET", "/stores/mine"),
    ("GET", "/stores/{id}"),
    ("PATCH", "/stores/{id}"),
    ("DELETE", "/stores/{id}"),
    ("GET", "/stores/{id}/products"),
    ("POST", "/stores/{id}/products"),
    ("GET", "/stores/{id}/orders"),
    ("GET", "/stores/{id}/payout-account"),
    ("PUT", "/stores/{id}/payout-account"),
    ("GET", "/stores/{id}/reviews"),
    ("POST", "/seller/redeem"),
    ("GET", "/cart"),
    ("DELETE", "/cart"),
    ("PUT", "/cart/items"),
    ("POST", "/checkout/delivery-options"),
    ("POST", "/checkout"),
    ("GET", "/checkout/{id}"),
    ("GET", "/orders"),
    ("GET", "/orders/{id}"),
    ("GET", "/orders/{id}/tracking"),
    ("POST", "/orders/{id}/cancel"),
    ("POST", "/orders/{id}/received"),
    ("POST", "/orders/{id}/ready"),
    ("POST", "/orders/{id}/picked-up"),
    ("POST", "/orders/{id}/out-for-delivery"),
    ("POST", "/orders/{id}/delivered"),
    ("POST", "/orders/{id}/dispute"),
    ("POST", "/orders/{id}/review"),
    ("GET", "/disputes/{id}"),
    ("POST", "/disputes/{id}/response"),
    ("POST", "/disputes/{id}/return-received"),
    ("POST", "/disputes/{id}/withdraw"),
    ("POST", "/reviews/{id}/reply"),
    ("GET", "/chats"),
    ("POST", "/chats"),
    ("GET", "/chats/ws"),
    ("GET", "/chats/{id}/messages"),
    ("POST", "/chats/{id}/messages"),
    ("POST", "/chats/{id}/read"),
];

/// `(METHOD, path)` for every operation under `paths:` in the file.
fn documented() -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    let mut in_paths = false;
    let mut current: Option<String> = None;
    for line in SPEC.lines() {
        if line.starts_with("paths:") {
            in_paths = true;
            continue;
        }
        if !in_paths {
            continue;
        }
        if !line.starts_with(' ') && !line.is_empty() {
            break; // another top-level key
        }
        if let Some(rest) = line.strip_prefix("  /") {
            current = Some(format!("/{}", rest.trim_end_matches(':').trim()));
        } else if let Some(m) = line.strip_prefix("    ") {
            let name = m.split(':').next().unwrap_or("");
            if ["get", "post", "put", "patch", "delete"].contains(&name)
                && !m.starts_with(' ')
                && let Some(p) = &current
            {
                out.insert((name.to_uppercase(), p.clone()));
            }
        }
    }
    out
}

fn concrete(path: &str) -> String {
    // Any `{name}` becomes a real-looking id, which every route's parser accepts.
    let mut out = String::new();
    let mut rest = path;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        out.push_str("11111111-1111-1111-1111-111111111111");
        rest = &rest[rest.find('}').unwrap() + 1..];
    }
    out.push_str(rest);
    out
}

#[test]
fn the_file_parses_into_the_operations_we_expect() {
    let docs = documented();
    assert!(docs.len() >= 55, "only {} operations found", docs.len());
    assert!(docs.contains(&("GET".into(), "/products".into())));
    assert!(docs.contains(&("POST".into(), "/uploads".into())));
    assert!(SPEC.starts_with("openapi: 3.1.0"));
}

#[test]
fn every_route_the_app_uses_is_documented() {
    let docs = documented();
    let missing: Vec<String> = APP_ROUTES
        .iter()
        .filter(|(m, p)| !docs.contains(&((*m).to_string(), (*p).to_string())))
        .map(|(m, p)| format!("{m} {p}"))
        .collect();
    assert!(
        missing.is_empty(),
        "missing from app/openapi.yaml: {missing:?}"
    );
}

#[test]
fn nothing_is_documented_that_the_app_does_not_use() {
    // So a typo or a stale route cannot hide behind the other check.
    let used: BTreeSet<(String, String)> = APP_ROUTES
        .iter()
        .map(|(m, p)| ((*m).to_string(), (*p).to_string()))
        .collect();
    let extra: Vec<_> = documented().difference(&used).cloned().collect();
    assert!(
        extra.is_empty(),
        "documented but not in APP_ROUTES: {extra:?}"
    );
}

#[sqlx::test]
async fn every_documented_route_exists_on_the_server(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let mut problems = Vec::new();
    for (method, path) in documented() {
        let url = concrete(&path);
        // The path: a method nobody implements is "not allowed" where the path exists, "not found" where not.
        let probe = app
            .request(Method::from_bytes(b"TRACE").unwrap(), &url, None, None)
            .await;
        if probe.status != 405 {
            problems.push(format!(
                "{method} {path}: no such path (TRACE answered {})",
                probe.status
            ));
            continue;
        }
        // The method: a documented method must not be refused as "not allowed".
        let res = app
            .request(
                Method::from_bytes(method.as_bytes()).unwrap(),
                &url,
                None,
                None,
            )
            .await;
        if res.status == 405 {
            problems.push(format!(
                "{method} {path}: the server has the path but not this method"
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[sqlx::test]
async fn the_router_check_can_actually_fail(pool: PgPool) {
    // Guard against the check above passing for the wrong reason.
    let app = TestApp::new(pool).await;
    let none = app
        .request(
            Method::from_bytes(b"TRACE").unwrap(),
            "/no/such/route",
            None,
            None,
        )
        .await;
    assert_eq!(none.status, 404);
    let wrong_method = app.request(Method::DELETE, "/config", None, None).await;
    assert_eq!(wrong_method.status, 405);
}
