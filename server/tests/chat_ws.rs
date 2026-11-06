//! The live socket, over a real local connection.

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::http::Method;
use common::{Listing, TestApp, TestResponse};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use server::{
    chat::{held, hub::MAX_SOCKETS_PER_USER},
    moderation::tone::ToneError,
};
use sqlx::PgPool;
use tokio::{net::TcpStream, time::timeout};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{self, Message, client::IntoClientRequest},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const SELLER: &str = "08055555551";
const BUYER: &str = "08022222221";
const STRANGER: &str = "08033333331";

struct Live {
    app: TestApp,
    addr: SocketAddr,
    shop: Listing,
    buyer: String,
    chat: Uuid,
}

async fn live(pool: PgPool) -> Live {
    let app = TestApp::new(pool).await;
    let addr = app.spawn_server().await;
    let shop = app
        .make_active_store_with_product(SELLER, "Kano Fabrics", 10_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let res = app
        .request(
            Method::POST,
            "/chats",
            Some(json!({ "product_id": shop.product_id })),
            Some(&buyer),
        )
        .await;
    let chat = res.json["id"].as_str().unwrap().parse().unwrap();
    Live {
        app,
        addr,
        shop,
        buyer,
        chat,
    }
}

async fn try_connect(
    addr: SocketAddr,
    path: &str,
    token: Option<&str>,
) -> Result<Socket, tungstenite::Error> {
    let mut request = format!("ws://{addr}{path}").into_client_request().unwrap();
    if let Some(token) = token {
        request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
    }
    connect_async(request).await.map(|(socket, _)| socket)
}

async fn connect(addr: SocketAddr, token: &str) -> Socket {
    try_connect(addr, "/chats/ws", Some(token)).await.unwrap()
}

fn status_of(error: tungstenite::Error) -> u16 {
    match error {
        tungstenite::Error::Http(res) => res.status().as_u16(),
        other => panic!("{other}"),
    }
}

/// The next JSON frame. Fails rather than hangs.
async fn next_json(socket: &mut Socket) -> Value {
    loop {
        let message = timeout(Duration::from_secs(3), socket.next())
            .await
            .expect("no frame arrived")
            .expect("the socket ended")
            .expect("socket error");
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// True when no text frame arrives in a short while.
async fn stays_quiet(socket: &mut Socket) -> bool {
    timeout(Duration::from_millis(300), async {
        while let Some(Ok(message)) = socket.next().await {
            if message.is_text() {
                return;
            }
        }
        std::future::pending::<()>().await
    })
    .await
    .is_err()
}

/// Waits for the server to end the socket.
async fn assert_closes(socket: &mut Socket, within: Duration) {
    timeout(within, async {
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                return;
            }
        }
    })
    .await
    .expect("the socket stayed open");
}

impl Live {
    async fn send(&self, token: &str, body: &str) -> TestResponse {
        self.app
            .request(
                Method::POST,
                &format!("/chats/{}/messages", self.chat),
                Some(json!({ "client_message_id": Uuid::new_v4(), "body": body })),
                Some(token),
            )
            .await
    }
}

#[sqlx::test]
async fn upgrade_needs_a_bearer_header(pool: PgPool) {
    let l = live(pool).await;
    let none = try_connect(l.addr, "/chats/ws", None).await.err().unwrap();
    assert_eq!(status_of(none), 401);
    let path = format!("/chats/ws?token={}", l.buyer);
    let in_url = try_connect(l.addr, &path, None).await.err().unwrap();
    assert_eq!(status_of(in_url), 401, "a token in the URL must be ignored");
}

#[sqlx::test]
async fn plain_request_gets_the_json_envelope(pool: PgPool) {
    let l = live(pool).await;
    let res = l
        .app
        .request(Method::GET, "/chats/ws", None, Some(&l.buyer))
        .await;
    assert_eq!(res.status, 422);
    assert!(res.json["error"]["code"].is_string(), "{}", res.json);
}

#[sqlx::test]
async fn sixth_socket_gets_429(pool: PgPool) {
    let l = live(pool).await;
    let mut open = Vec::new();
    for _ in 0..MAX_SOCKETS_PER_USER {
        open.push(connect(l.addr, &l.buyer).await);
    }
    let refused = try_connect(l.addr, "/chats/ws", Some(&l.buyer))
        .await
        .err()
        .unwrap();
    assert_eq!(status_of(refused), 429);
}

#[sqlx::test]
async fn message_arrives_live_for_both_parties(pool: PgPool) {
    let l = live(pool).await;
    let mut seller = connect(l.addr, &l.shop.seller_token).await;
    let mut buyer = connect(l.addr, &l.buyer).await;
    l.send(&l.buyer, "hello").await;
    let frame = next_json(&mut seller).await;
    assert_eq!(frame["type"], "message");
    assert_eq!(frame["body"], "hello");
    assert_eq!(frame["from"], "buyer");
    assert_eq!(next_json(&mut buyer).await["seq"], 1);
}

#[sqlx::test]
async fn socket_only_gets_own_conversations(pool: PgPool) {
    let l = live(pool).await;
    let (_, stranger) = l.app.sign_up_verified(STRANGER).await;
    let mut third = connect(l.addr, &stranger).await;
    l.send(&l.buyer, "hello").await;
    assert!(stays_quiet(&mut third).await);
}

#[sqlx::test]
async fn flagged_text_never_reaches_the_recipient_socket(pool: PgPool) {
    let l = live(pool).await;
    let mut seller = connect(l.addr, &l.shop.seller_token).await;
    let res = l.send(&l.buyer, "call me on 08031234567").await;
    assert_eq!(res.json["outcome"], "blocked");
    assert!(stays_quiet(&mut seller).await);
}

#[sqlx::test]
async fn sender_gets_held_then_outcome_frame(pool: PgPool) {
    let l = live(pool).await;
    let mut buyer = connect(l.addr, &l.buyer).await;
    l.app.tone.set_next(Err(ToneError::Unavailable));
    assert_eq!(l.send(&l.buyer, "hello").await.json["outcome"], "held");
    let frame = next_json(&mut buyer).await;
    assert_eq!(frame["type"], "moderation");
    assert_eq!(frame["outcome"]["outcome"], "held");

    sqlx::query("UPDATE chat_messages SET next_attempt_at = now() - interval '1 second'")
        .execute(&l.app.pool)
        .await
        .unwrap();
    held::run(&l.app.state, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(next_json(&mut buyer).await["type"], "message");
    let frame = next_json(&mut buyer).await;
    assert_eq!(frame["type"], "moderation");
    assert_eq!(frame["outcome"]["outcome"], "delivered");
}

#[sqlx::test]
async fn revoked_session_is_disconnected(pool: PgPool) {
    let l = live(pool).await;
    let mut buyer = connect(l.addr, &l.buyer).await;
    sqlx::query("UPDATE sessions SET revoked_at = now()")
        .execute(&l.app.pool)
        .await
        .unwrap();
    assert_closes(&mut buyer, Duration::from_secs(2)).await;
}

#[sqlx::test]
async fn deleted_account_is_disconnected(pool: PgPool) {
    let l = live(pool).await;
    let mut buyer = connect(l.addr, &l.buyer).await;
    sqlx::query("UPDATE users SET deleted_at = now()")
        .execute(&l.app.pool)
        .await
        .unwrap();
    assert_closes(&mut buyer, Duration::from_secs(2)).await;
}

#[sqlx::test]
async fn hub_stop_closes_sockets(pool: PgPool) {
    let l = live(pool).await;
    let mut buyer = connect(l.addr, &l.buyer).await;
    let mut seller = connect(l.addr, &l.shop.seller_token).await;
    l.app.state.chat_hub.stop();
    assert_closes(&mut buyer, Duration::from_secs(2)).await;
    assert_closes(&mut seller, Duration::from_secs(2)).await;
}

#[sqlx::test]
async fn incoming_frames_cannot_send_a_message(pool: PgPool) {
    let l = live(pool).await;
    let mut buyer = connect(l.addr, &l.buyer).await;
    let attempt = json!({ "client_message_id": Uuid::new_v4(), "body": "over the socket" });
    buyer
        .send(Message::text(attempt.to_string()))
        .await
        .unwrap();
    assert!(stays_quiet(&mut buyer).await);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM chat_messages")
        .fetch_one(&l.app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test]
async fn oversized_incoming_frame_closes_the_socket(pool: PgPool) {
    let l = live(pool).await;
    let mut buyer = connect(l.addr, &l.buyer).await;
    buyer.send(Message::text("x".repeat(5000))).await.unwrap();
    assert_closes(&mut buyer, Duration::from_secs(2)).await;
}

#[sqlx::test]
async fn missed_frames_are_recoverable(pool: PgPool) {
    let l = live(pool).await;
    let mut seller = connect(l.addr, &l.shop.seller_token).await;
    l.send(&l.buyer, "one").await;
    assert_eq!(next_json(&mut seller).await["seq"], 1);
    seller.close(None).await.unwrap();
    drop(seller);

    l.send(&l.buyer, "two").await;
    let res = l
        .app
        .request(
            Method::GET,
            &format!("/chats/{}/messages?after=1", l.chat),
            None,
            Some(&l.shop.seller_token),
        )
        .await;
    assert_eq!(res.json["messages"][0]["body"], "two");
    assert_eq!(res.json["messages"].as_array().unwrap().len(), 1);
}
