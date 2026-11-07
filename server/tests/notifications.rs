//! The notification outbox, inbox and push devices (plan 5, task 7).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use server::notify::{
    kinds::Kind,
    service::{Notice, enqueue},
};
use sqlx::PgPool;
use uuid::Uuid;

const ADA: &str = "08022222221";
const BOLA: &str = "08033333331";

async fn notice(app: &TestApp, n: Notice) {
    let mut conn = app.pool.acquire().await.unwrap();
    enqueue(&mut conn, n).await.unwrap();
}

async fn inbox(app: &TestApp, token: &str, query: &str) -> TestResponse {
    app.request(
        Method::GET,
        &format!("/me/notifications{query}"),
        None,
        Some(token),
    )
    .await
}

async fn read(app: &TestApp, token: &str, body: Value) -> TestResponse {
    app.request(
        Method::POST,
        "/me/notifications/read",
        Some(body),
        Some(token),
    )
    .await
}

async fn register(app: &TestApp, token: &str, push_token: &str) -> TestResponse {
    app.request(
        Method::PUT,
        "/me/push-device",
        Some(json!({ "token": push_token, "platform": "android" })),
        Some(token),
    )
    .await
}

/// Inserts `n` order notices for `user`, one second apart, oldest first.
async fn many(app: &TestApp, user: Uuid, n: usize) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for i in 0..n {
        let order = Uuid::new_v4();
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO notifications (user_id, kind, dedupe_key, created_at) \
             VALUES ($1, 'order_paid', $2, now() - make_interval(secs => $3)) RETURNING id",
        )
        .bind(user)
        .bind(format!("order_paid:{order}"))
        .bind((n - i) as f64)
        .fetch_one(&app.pool)
        .await
        .unwrap();
        ids.push(id);
    }
    ids
}

#[sqlx::test]
async fn enqueue_is_idempotent(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    let (ada, order) = (o.buyer_id, o.order_id);
    notice(&app, Notice::order(ada, Kind::OrderPaid, order)).await;
    notice(&app, Notice::order(ada, Kind::OrderPaid, order)).await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1")
        .bind(ada)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn inbox_pages_newest_first_and_counts_unread(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, token) = app.sign_up_verified(ADA).await;
    let ids = many(&app, ada, 5).await;
    let page = inbox(&app, &token, "?limit=2").await;
    assert_eq!(page.status, 200, "{}", page.json);
    assert_eq!(page.json["unread"], 5);
    let items = page.json["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], ids[4].to_string());
    assert_eq!(items[1]["id"], ids[3].to_string());
    assert_eq!(items[0]["kind"], "order_paid");
    assert_eq!(items[0]["read"], false);

    let last = items[1]["id"].as_str().unwrap();
    let next = inbox(&app, &token, &format!("?limit=2&before={last}")).await;
    let items = next.json["items"].as_array().unwrap();
    assert_eq!(items[0]["id"], ids[2].to_string());
    assert_eq!(items[1]["id"], ids[1].to_string());

    assert_eq!(
        read(&app, &token, json!({ "ids": [ids[4]] })).await.status,
        204
    );
    let page = inbox(&app, &token, "").await;
    assert_eq!(page.json["unread"], 4);
    assert_eq!(page.json["items"].as_array().unwrap().len(), 5);
    assert_eq!(page.json["items"][0]["read"], true);

    for bad in ["?limit=0", "?limit=101", "?before=nope"] {
        assert_eq!(inbox(&app, &token, bad).await.status, 422, "{bad}");
    }
}

#[sqlx::test]
async fn inbox_copy_comes_from_kind(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    let (ada, token) = (o.buyer_id, o.buyer_token.clone());
    for kind in Kind::ALL {
        notice(&app, Notice::order(ada, kind, o.order_id)).await;
    }
    let page = inbox(&app, &token, "?limit=100").await;
    let items = page.json["items"].as_array().unwrap();
    assert_eq!(items.len(), 14);
    for kind in Kind::ALL {
        let item = items
            .iter()
            .find(|i| i["kind"] == kind.as_str())
            .unwrap_or_else(|| panic!("no {} item", kind.as_str()));
        assert_eq!(item["title"], kind.title());
        assert_eq!(item["body"], kind.body());
    }
    let paid = items.iter().find(|i| i["kind"] == "order_paid").unwrap();
    assert_eq!(paid["title"], "Paid \u{2014} please ship");
    // The table holds no text.
    let rows: Vec<String> = sqlx::query_scalar("SELECT to_jsonb(t)::text FROM notifications t")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    for raw in rows {
        assert!(!raw.contains("Paid"), "copy stored in the table");
    }
}

#[sqlx::test]
async fn users_only_read_and_mark_their_own(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, ada_token) = app.sign_up_verified(ADA).await;
    let (bola, bola_token) = app.sign_up_verified(BOLA).await;
    let mine = many(&app, ada, 1).await;
    let theirs = many(&app, bola, 1).await;

    let page = inbox(&app, &ada_token, "").await;
    assert_eq!(page.json["items"].as_array().unwrap().len(), 1);
    assert_eq!(page.json["items"][0]["id"], mine[0].to_string());

    // Marking someone else's notice changes nothing.
    assert_eq!(
        read(&app, &ada_token, json!({ "ids": [theirs[0]] }))
            .await
            .status,
        204
    );
    assert_eq!(inbox(&app, &bola_token, "").await.json["unread"], 1);
    // A cursor from another user's inbox does not leak or page.
    let page = inbox(&app, &ada_token, &format!("?before={}", theirs[0])).await;
    assert_eq!(page.json["items"].as_array().unwrap().len(), 0);

    assert_eq!(
        read(&app, &ada_token, json!({ "all": true })).await.status,
        204
    );
    assert_eq!(inbox(&app, &ada_token, "").await.json["unread"], 0);
    assert_eq!(inbox(&app, &bola_token, "").await.json["unread"], 1);
}

#[sqlx::test]
async fn read_body_needs_exactly_one_mode(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, token) = app.sign_up_verified(ADA).await;
    let ids = many(&app, ada, 1).await;
    for body in [
        json!({}),
        json!({ "ids": [ids[0]], "all": true }),
        json!({ "all": false }),
        json!({ "ids": [] }),
        json!({ "ids": vec![ids[0]; 101] }),
        json!({ "ids": [ids[0]], "extra": 1 }),
    ] {
        assert_eq!(read(&app, &token, body.clone()).await.status, 422, "{body}");
    }
    assert_eq!(inbox(&app, &token, "").await.json["unread"], 1);
    assert_eq!(
        read(&app, &token, json!({ "ids": vec![ids[0]; 100] }))
            .await
            .status,
        204
    );
}

#[sqlx::test]
async fn device_token_is_encrypted_and_deduped(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, ada_token) = app.sign_up_verified(ADA).await;
    let (bola, bola_token) = app.sign_up_verified(BOLA).await;
    let push = "fcm-token-Zq81xK_secret:APA91b";

    assert_eq!(register(&app, &ada_token, push).await.status, 204);
    // The same caller registering again is idempotent but still audited.
    assert_eq!(register(&app, &ada_token, push).await.status, 204);
    let (n, owner): (i64, Uuid) =
        sqlx::query_as("SELECT count(*), min(user_id::text)::uuid FROM push_devices")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!((n, owner), (1, ada));
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'push_device.registered'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(audits, 2);

    // Another account registering the same token takes the row.
    assert_eq!(register(&app, &bola_token, push).await.status, 204);
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as("SELECT user_id, session_id FROM push_devices")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, bola);
    let bola_session: Uuid = sqlx::query_scalar("SELECT id FROM sessions WHERE user_id = $1")
        .bind(bola)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows[0].1, bola_session);

    for table in ["push_devices", "audit_log", "sessions", "users"] {
        let rows: Vec<String> =
            sqlx::query_scalar(&format!("SELECT to_jsonb(t)::text FROM {table} t"))
                .fetch_all(&app.pool)
                .await
                .unwrap();
        for raw in rows {
            assert!(!raw.contains("secret"), "plaintext token in {table}");
            assert!(!raw.contains("APA91b"), "plaintext token in {table}");
        }
    }

    // Validation.
    for body in [
        json!({ "token": "", "platform": "android" }),
        json!({ "token": "has space", "platform": "android" }),
        json!({ "token": "x".repeat(4097), "platform": "android" }),
        json!({ "token": "ok", "platform": "windows" }),
        json!({ "token": "ok" }),
    ] {
        let res = app
            .request(Method::PUT, "/me/push-device", Some(body), Some(&ada_token))
            .await;
        assert_eq!(res.status, 422);
    }
    assert_eq!(
        register(&app, &ada_token, &"x".repeat(4096)).await.status,
        204
    );
}

#[sqlx::test]
async fn delete_device_disables_current_session_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (ada, first) = app.sign_up_verified(ADA).await;
    let second = app.sign_in(ADA, common::TEST_PASSWORD).await.json["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(first, second);
    assert_eq!(register(&app, &first, "token-on-first").await.status, 204);
    assert_eq!(register(&app, &second, "token-on-second").await.status, 204);

    let res = app
        .request(Method::DELETE, "/me/push-device", None, Some(&second))
        .await;
    assert_eq!(res.status, 204);
    let live: Vec<Uuid> = sqlx::query_scalar(
        "SELECT session_id FROM push_devices WHERE user_id = $1 AND disabled_at IS NULL",
    )
    .bind(ada)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(live.len(), 1);
    let first_session: Uuid = sqlx::query_scalar("SELECT id FROM sessions WHERE token_hash = $1")
        .bind(server::auth::sessions::hash_token(&first))
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(live[0], first_session);
    let removed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'push_device.removed'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(removed, 1);

    // Registering again re-enables it.
    assert_eq!(register(&app, &second, "token-on-second").await.status, 204);
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM push_devices WHERE user_id = $1 AND disabled_at IS NULL",
    )
    .bind(ada)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(live, 2);
}

#[sqlx::test]
async fn reading_a_chat_marks_its_message_notices_read(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = app
        .make_active_store_with_product("08055555551", "Kano Fabrics", 10_000, 5)
        .await;
    let (buyer, buyer_token) = app.sign_up_verified(ADA).await;
    let seller = shop.seller_id;
    let chat = app
        .request(
            Method::POST,
            "/chats",
            Some(json!({ "product_id": shop.product_id })),
            Some(&buyer_token),
        )
        .await;
    assert_eq!(chat.status, 200, "{}", chat.json);
    let chat: Uuid = chat.json["id"].as_str().unwrap().parse().unwrap();
    for (user, key) in [(buyer, "a"), (buyer, "b"), (seller, "c")] {
        sqlx::query(
            "INSERT INTO notifications (user_id, kind, conversation_id, dedupe_key) \
             VALUES ($1, 'chat_message', $2, $3)",
        )
        .bind(user)
        .bind(chat)
        .bind(format!("chat_message:{chat}:{key}"))
        .execute(&app.pool)
        .await
        .unwrap();
    }
    let res = app
        .request(
            Method::POST,
            &format!("/chats/{chat}/read"),
            Some(json!({ "seq": 0 })),
            Some(&buyer_token),
        )
        .await;
    assert_eq!(res.status, 204);
    assert_eq!(inbox(&app, &buyer_token, "").await.json["unread"], 0);
    let seller_unread: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND read_at IS NULL",
    )
    .bind(seller)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(seller_unread, 1);
}
