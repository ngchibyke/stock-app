//! The phone app's config and the notice routes (plan 7, S3).

mod common;

use axum::http::Method;
use common::TestApp;
use serde_json::Value;
use server::notify::{
    kinds::Kind,
    service::{Notice, enqueue},
};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test]
async fn config_is_public_and_says_what_the_app_needs(pool: PgPool) {
    let app = TestApp::new(pool).await;
    // No sign-in: the app reads it before anyone has signed in.
    let res = app.request(Method::GET, "/config", None, None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["photo_base_url"], "https://photos.test");
    assert_eq!(res.json["min_app_version"], "1.0.0");
    assert_eq!(res.json["fct_only"], true);
    assert!(
        res.json["payment_return_url"]
            .as_str()
            .is_some_and(|u| !u.is_empty())
    );
    // Nothing secret or personal.
    let text = res.json.to_string();
    for word in ["key", "secret", "token", "password", "database"] {
        assert!(!text.to_lowercase().contains(word), "{word} in {text}");
    }
}

#[sqlx::test]
async fn without_photo_storage_the_base_url_is_null(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let mut state = app.state.clone();
    state.app_config = std::sync::Arc::new(server::app_config::AppConfig {
        photo_base_url: None,
        min_app_version: "2.1.0".into(),
    });
    let router = server::build_app(state);
    use tower::ServiceExt;
    let res = router
        .oneshot(
            axum::http::Request::builder()
                .uri("/config")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    use http_body_util::BodyExt;
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["photo_base_url"].is_null());
    assert_eq!(json["min_app_version"], "2.1.0");
}

async fn notice(app: &TestApp, n: Notice) {
    let mut conn = app.pool.acquire().await.unwrap();
    enqueue(&mut conn, n).await.unwrap();
}

#[sqlx::test]
async fn inbox_items_carry_the_route_a_tap_opens(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let o = app.shipped_order("self", "delivered").await;
    let buyer = o.buyer_id;
    let order = o.order_id;
    let conversation: Uuid = {
        // A real conversation row, so the foreign keys hold.
        let res = app
            .request(
                Method::POST,
                "/chats",
                Some(serde_json::json!({ "order_id": order })),
                Some(&o.buyer_token),
            )
            .await;
        assert_eq!(res.status, 200, "{}", res.json);
        res.json["id"].as_str().unwrap().parse().unwrap()
    };

    notice(&app, Notice::order(buyer, Kind::OrderOutForDelivery, order)).await;
    notice(
        &app,
        Notice::user(buyer, Kind::ModerationMuted, "mute:1".into()),
    )
    .await;
    let mut chat = Notice::user(buyer, Kind::ChatMessage, "chat:1".into());
    chat.conversation_id = Some(conversation);
    notice(&app, chat).await;

    let res = app
        .request(Method::GET, "/me/notifications", None, Some(&o.buyer_token))
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let route_of = |kind: &str| -> Value {
        res.json["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind}: {}", res.json))["route"]
            .clone()
    };
    assert_eq!(route_of("order_out_for_delivery"), format!("order:{order}"));
    assert_eq!(route_of("moderation_muted"), "account");
    assert_eq!(route_of("chat_message"), format!("chat:{conversation}"));
}

#[sqlx::test]
async fn a_push_carries_the_route_and_still_no_personal_data(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let o = app.shipped_order("self", "delivered").await;
    let res = app
        .request(
            Method::PUT,
            "/me/push-device",
            Some(serde_json::json!({ "token": "buyer-phone", "platform": "android" })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(res.status, 204, "{}", res.json);
    notice(
        &app,
        Notice::order(o.buyer_id, Kind::OrderDelivered, o.order_id),
    )
    .await;
    server::notify::worker::run_once(&app.state).await.unwrap();
    let seen = app.push.seen();
    let m = seen
        .iter()
        .find(|m| m.kind == "order_delivered")
        .expect("pushed");
    assert_eq!(m.route, format!("order:{}", o.order_id));
    // The message is still fixed copy.
    assert_eq!(m.title, Kind::OrderDelivered.title());
    assert_eq!(m.body, Kind::OrderDelivered.body());
}
