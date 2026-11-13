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
    // The fixture's own steps raise the buyer's order notices; count this kind.
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND kind = 'order_paid'",
    )
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

// ---- Notices raised at each event, and the reminders job (plan 5, task 8) ----

use common::ShippedOrder;
use server::{
    moderation::{filter::Category, tone::ToneVerdict},
    notify::reminders,
};
use tokio_util::sync::CancellationToken;

/// The user's notices, oldest first: `(kind, order, dispute, conversation)`.
async fn notices(
    app: &TestApp,
    user: Uuid,
) -> Vec<(String, Option<Uuid>, Option<Uuid>, Option<Uuid>)> {
    sqlx::query_as(
        "SELECT kind, order_id, dispute_id, conversation_id FROM notifications \
         WHERE user_id = $1 ORDER BY created_at, id",
    )
    .bind(user)
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

/// The user's notice kinds, sorted by name.
async fn kinds(app: &TestApp, user: Uuid) -> Vec<String> {
    let mut k: Vec<String> = notices(app, user).await.into_iter().map(|n| n.0).collect();
    k.sort();
    k
}

async fn clear(app: &TestApp) {
    sqlx::query("DELETE FROM notifications")
        .execute(&app.pool)
        .await
        .unwrap();
}

async fn post(app: &TestApp, path: &str, body: Value, token: &str) -> TestResponse {
    app.request(Method::POST, path, Some(body), Some(token))
        .await
}

async fn open_dispute(app: &TestApp, o: &ShippedOrder) -> String {
    let res = post(
        app,
        &format!("/orders/{}/dispute", o.order_id),
        json!({ "reason": "damaged", "body": "The zip was torn when it arrived", "photo_keys": [] }),
        &o.buyer_token,
    )
    .await;
    assert_eq!(res.status, 201, "{}", res.json);
    res.json["id"].as_str().unwrap().to_string()
}

async fn answered_dispute(app: &TestApp, o: &ShippedOrder) -> String {
    let id = open_dispute(app, o).await;
    let res = post(
        app,
        &format!("/disputes/{id}/response"),
        json!({ "body": "It left the shop in perfect condition", "photo_keys": [] }),
        &o.seller_token,
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    id
}

#[sqlx::test]
async fn each_event_raises_its_notice_order_steps(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    // Payment: the seller alone. Each step after it: the buyer alone.
    assert_eq!(
        notices(&app, o.seller_id).await,
        vec![("order_paid".to_string(), Some(o.order_id), None, None)]
    );
    assert_eq!(
        notices(&app, o.buyer_id)
            .await
            .into_iter()
            .map(|n| (n.0, n.1))
            .collect::<Vec<_>>(),
        vec![
            ("order_picked_up".to_string(), Some(o.order_id)),
            ("order_out_for_delivery".to_string(), Some(o.order_id)),
            ("order_delivered".to_string(), Some(o.order_id)),
        ]
    );
}

#[sqlx::test]
async fn handover_delivery_sends_no_delivered_notice(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("self", "delivered").await;
    assert_eq!(
        kinds(&app, o.buyer_id).await,
        ["order_out_for_delivery", "order_picked_up"]
    );
}

#[sqlx::test]
async fn each_event_raises_its_notice_release(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    clear(&app).await;
    let res = post(
        &app,
        &format!("/orders/{}/received", o.order_id),
        json!({ "confirm": true }),
        &o.buyer_token,
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(
        notices(&app, o.seller_id).await,
        vec![("funds_released".to_string(), Some(o.order_id), None, None)]
    );
    assert!(kinds(&app, o.buyer_id).await.is_empty());
}

#[sqlx::test]
async fn each_event_raises_its_notice_dispute_and_full_refund(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.make_owner().await;
    let o = app.shipped_order("courier", "delivered").await;
    clear(&app).await;

    let id = answered_dispute(&app, &o).await;
    let dispute: Uuid = id.parse().unwrap();
    assert_eq!(
        notices(&app, o.seller_id).await,
        vec![("dispute_opened".to_string(), None, Some(dispute), None)]
    );
    assert!(kinds(&app, o.buyer_id).await.is_empty());

    let res = post(
        &app,
        &format!("/admin/disputes/{id}/request-return"),
        json!({ "paid_by": "seller", "note": "Item must come back first" }),
        &owner,
    )
    .await;
    assert!(res.status < 300, "{}", res.json);
    assert_eq!(kinds(&app, o.buyer_id).await, ["return_requested"]);
    assert_eq!(
        kinds(&app, o.seller_id).await,
        ["dispute_opened", "return_requested"]
    );

    let res = post(
        &app,
        &format!("/admin/disputes/{id}/decide"),
        json!({ "decision": "full_refund", "note": "Photos confirm the damage" }),
        &owner,
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(
        kinds(&app, o.buyer_id).await,
        ["dispute_decided", "refund_started", "return_requested"]
    );
    assert_eq!(
        kinds(&app, o.seller_id).await,
        ["dispute_decided", "dispute_opened", "return_requested"]
    );
    let refund = notices(&app, o.buyer_id)
        .await
        .into_iter()
        .find(|n| n.0 == "refund_started")
        .unwrap();
    assert_eq!(refund.1, Some(o.order_id));
}

#[sqlx::test]
async fn each_event_raises_its_notice_partial_refund(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.make_owner().await;
    let o = app.shipped_order("courier", "delivered").await;
    let id = answered_dispute(&app, &o).await;
    clear(&app).await;
    let res = post(
        &app,
        &format!("/admin/disputes/{id}/decide"),
        json!({ "decision": "partial_refund", "refund_kobo": 2_000, "note": "Split it" }),
        &owner,
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(
        kinds(&app, o.buyer_id).await,
        ["dispute_decided", "refund_started"]
    );
    assert_eq!(
        kinds(&app, o.seller_id).await,
        ["dispute_decided", "funds_released"]
    );
}

/// A product chat between a fresh seller and buyer:
/// `(conversation, seller id, seller token, buyer id, buyer token)`.
async fn chat_pair(app: &TestApp) -> (Uuid, Uuid, String, Uuid, String) {
    let shop = app
        .make_active_store_with_product("08055555551", "Kano Fabrics", 10_000, 5)
        .await;
    let (buyer_id, buyer) = app.sign_up_verified(ADA).await;
    let res = post(
        app,
        "/chats",
        json!({ "product_id": shop.product_id }),
        &buyer,
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let chat = res.json["id"].as_str().unwrap().parse().unwrap();
    (chat, shop.seller_id, shop.seller_token, buyer_id, buyer)
}

async fn say(app: &TestApp, chat: Uuid, token: &str, body: &str) -> TestResponse {
    post(
        app,
        &format!("/chats/{chat}/messages"),
        json!({ "client_message_id": Uuid::new_v4(), "body": body }),
        token,
    )
    .await
}

#[sqlx::test]
async fn chat_notifications_coalesce_until_read(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (chat, seller_id, seller, buyer_id, buyer) = chat_pair(&app).await;
    for text in ["is it available?", "in blue?", "hello?"] {
        let res = say(&app, chat, &buyer, text).await;
        assert_eq!(res.json["outcome"], "delivered", "{}", res.json);
    }
    assert_eq!(
        notices(&app, seller_id).await,
        vec![("chat_message".to_string(), None, None, Some(chat))]
    );
    assert!(kinds(&app, buyer_id).await.is_empty(), "not the sender");

    let res = post(
        &app,
        &format!("/chats/{chat}/read"),
        json!({ "seq": 3 }),
        &seller,
    )
    .await;
    assert_eq!(res.status, 204);
    say(&app, chat, &buyer, "still there?").await;
    assert_eq!(
        kinds(&app, seller_id).await,
        ["chat_message", "chat_message"]
    );

    // The seller's reply goes to the buyer.
    say(&app, chat, &seller, "yes, in blue").await;
    assert_eq!(kinds(&app, buyer_id).await, ["chat_message"]);
}

#[sqlx::test]
async fn each_event_raises_its_notice_strike_and_mute(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (chat, seller_id, _seller, buyer_id, buyer) = chat_pair(&app).await;
    app.tone.set_next(Ok(ToneVerdict::serious(Category::Abuse)));
    let res = say(&app, chat, &buyer, "a subtle threat").await;
    assert_eq!(res.json["outcome"], "blocked", "{}", res.json);
    assert_eq!(kinds(&app, buyer_id).await, ["moderation_warning"]);
    assert!(kinds(&app, seller_id).await.is_empty());

    // The strike that reaches the mute threshold also tells them of the mute.
    sqlx::query("UPDATE moderation_strikes SET at = now() - interval '1 hour'")
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE platform_settings SET mute_after_strikes = 2")
        .execute(&app.pool)
        .await
        .unwrap();
    app.tone.set_next(Ok(ToneVerdict::serious(Category::Abuse)));
    say(&app, chat, &buyer, "another threat").await;
    assert_eq!(
        kinds(&app, buyer_id).await,
        [
            "moderation_muted",
            "moderation_warning",
            "moderation_warning"
        ]
    );
    assert!(kinds(&app, seller_id).await.is_empty());
}

/// A delivered courier order with a 72-hour release window.
async fn delivered_72(app: &TestApp) -> ShippedOrder {
    sqlx::query("UPDATE platform_settings SET release_after_hours = 72")
        .execute(&app.pool)
        .await
        .unwrap();
    let o = app.shipped_order("courier", "delivered").await;
    clear(app).await;
    o
}

/// Moves the order so that it was delivered `hours` ago (72-hour window).
async fn delivered_ago(app: &TestApp, order: Uuid, hours: i32) {
    sqlx::query(
        "UPDATE orders SET delivered_at = now() - make_interval(hours => $2), \
                release_at = now() - make_interval(hours => $2) + interval '72 hours' \
         WHERE id = $1",
    )
    .bind(order)
    .bind(hours)
    .execute(&app.pool)
    .await
    .unwrap();
}

async fn remind(app: &TestApp) -> u32 {
    reminders::run(&app.state, &CancellationToken::new())
        .await
        .unwrap()
        .done
}

#[sqlx::test]
async fn reminders_follow_the_window(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = delivered_72(&app).await;

    delivered_ago(&app, o.order_id, 23).await;
    assert_eq!(remind(&app).await, 0);
    assert!(kinds(&app, o.buyer_id).await.is_empty());

    delivered_ago(&app, o.order_id, 25).await;
    assert_eq!(remind(&app).await, 1);
    assert_eq!(remind(&app).await, 0, "once only");
    assert_eq!(
        notices(&app, o.buyer_id).await,
        vec![("confirm_reminder".to_string(), Some(o.order_id), None, None)]
    );

    delivered_ago(&app, o.order_id, 49).await;
    assert_eq!(remind(&app).await, 1);
    assert_eq!(remind(&app).await, 0, "once only");
    assert_eq!(
        kinds(&app, o.buyer_id).await,
        ["confirm_reminder", "release_tomorrow"]
    );

    delivered_ago(&app, o.order_id, 73).await;
    assert_eq!(remind(&app).await, 0);
    assert_eq!(
        kinds(&app, o.buyer_id).await,
        ["confirm_reminder", "release_tomorrow"]
    );
    assert!(kinds(&app, o.seller_id).await.is_empty());
}

#[sqlx::test]
async fn missed_stage_is_skipped_not_sent_late(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = delivered_72(&app).await;
    delivered_ago(&app, o.order_id, 49).await;
    assert_eq!(remind(&app).await, 1);
    assert_eq!(remind(&app).await, 0);
    assert_eq!(kinds(&app, o.buyer_id).await, ["release_tomorrow"]);
}

#[sqlx::test]
async fn nothing_is_sent_once_the_release_is_due(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = delivered_72(&app).await;
    delivered_ago(&app, o.order_id, 73).await;
    assert_eq!(remind(&app).await, 0);
    assert!(kinds(&app, o.buyer_id).await.is_empty());
}

#[sqlx::test]
async fn short_window_has_no_next_day_reminder(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = delivered_72(&app).await;
    // A 36-hour window: "next day" would land inside the last 24 hours.
    sqlx::query(
        "UPDATE orders SET delivered_at = now() - interval '6 hours', \
                release_at = now() + interval '30 hours' WHERE id = $1",
    )
    .bind(o.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(remind(&app).await, 0);
    sqlx::query(
        "UPDATE orders SET delivered_at = now() - interval '25 hours', \
                release_at = now() + interval '11 hours' WHERE id = $1",
    )
    .bind(o.order_id)
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(remind(&app).await, 1);
    assert_eq!(kinds(&app, o.buyer_id).await, ["release_tomorrow"]);
}

#[sqlx::test]
async fn held_order_gets_no_reminder(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = delivered_72(&app).await;
    sqlx::query("UPDATE orders SET held_at = now(), hold_reason = 'check' WHERE id = $1")
        .bind(o.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    delivered_ago(&app, o.order_id, 49).await;
    assert_eq!(remind(&app).await, 0);
    assert!(kinds(&app, o.buyer_id).await.is_empty());
}

#[sqlx::test]
async fn reminders_run_in_the_job_loop(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = delivered_72(&app).await;
    delivered_ago(&app, o.order_id, 25).await;
    let report = server::escrow::jobs::run_due_jobs(&app.state).await;
    assert_eq!(report.reminders_sent, 1);
    let runs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_runs WHERE job = 'confirm_reminders'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(runs, 1);
}

#[sqlx::test]
async fn rolled_back_change_leaves_no_notice(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("courier", "delivered").await;
    clear(&app).await;
    sqlx::query("UPDATE orders SET held_at = now(), hold_reason = 'check' WHERE id = $1")
        .bind(o.order_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let count = "SELECT count(*) FROM notifications WHERE kind = 'refund_started'";

    let mut tx = app.pool.begin().await.unwrap();
    server::escrow::refund::owner_begin_in(&mut tx, o.order_id)
        .await
        .unwrap();
    let inside: i64 = sqlx::query_scalar(count).fetch_one(&mut *tx).await.unwrap();
    assert_eq!(inside, 1, "raised with the refund");
    tx.rollback().await.unwrap();

    let after: i64 = sqlx::query_scalar(count)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(after, 0, "and gone with it");
}
