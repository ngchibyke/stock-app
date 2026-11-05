//! Conversations: who may open one, who may read one, and what each side sees.

mod common;

use axum::http::Method;
use common::{Listing, TestApp};
use serde_json::{Value, json};
use server::limits::NEW_CONVERSATIONS_PER_DAY;
use sqlx::PgPool;
use uuid::Uuid;

const SELLER: &str = "08055555551";
const BUYER: &str = "08022222221";
const STRANGER: &str = "08033333331";
const METHOD: &str = "mono_directpay";

async fn listing(app: &TestApp) -> Listing {
    app.make_active_store_with_product(SELLER, "Kano Fabrics", 10_000, 5)
        .await
}

async fn open(app: &TestApp, token: &str, body: Value) -> common::TestResponse {
    app.request(Method::POST, "/chats", Some(body), Some(token))
        .await
}

async fn product_chat(app: &TestApp, token: &str, product: Uuid) -> Uuid {
    let res = open(app, token, json!({ "product_id": product })).await;
    assert_eq!(res.status, 200, "{}", res.json);
    res.json["id"].as_str().unwrap().parse().unwrap()
}

/// A second active store for the same seller, with one product.
async fn second_store(app: &TestApp, shop: &Listing) -> Uuid {
    let res = app
        .request(
            Method::POST,
            "/stores",
            Some(json!({
                "name": "Kano Shoes",
                "pickup_address": "12 Unique Street, Wuse 2",
                "pickup_state": "FCT",
                "delivery_courier": true,
                "delivery_self": true,
                "self_delivery_fee_kobo": 0,
                "pickup_area_council": "amac",
                "business_phone": common::STORE_PHONE
            })),
            Some(&shop.seller_token),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let store: Uuid = res.json["id"].as_str().unwrap().parse().unwrap();
    sqlx::query("UPDATE stores SET status = 'active' WHERE id = $1")
        .bind(store)
        .execute(&app.pool)
        .await
        .unwrap();
    app.add_product(&shop.seller_token, store, 10_000, &[(json!({}), 1)])
        .await
}

/// A paid order from the listing's store.
async fn paid_order(app: &TestApp, buyer: &str, listing: &Listing) -> Uuid {
    assert_eq!(app.set_cart(buyer, listing.variant_id, 1).await.status, 200);
    let res = app.checkout(buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);
    let checkout: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
    let order: Uuid = sqlx::query_scalar("SELECT id FROM orders WHERE checkout_id = $1")
        .bind(checkout)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE orders SET status = 'paid', paid_at = now() WHERE id = $1")
        .bind(order)
        .execute(&app.pool)
        .await
        .unwrap();
    order
}

/// A delivered row, as sending will write it.
async fn delivered(app: &TestApp, conversation: Uuid, sender: Uuid, text: &str) -> i64 {
    let seq: i64 = sqlx::query_scalar(
        "UPDATE conversations SET last_seq = last_seq + 1 WHERE id = $1 RETURNING last_seq",
    )
    .bind(conversation)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chat_messages (conversation_id, sender_id, client_message_id, status, \
                seq, body_enc, delivered_at) \
         VALUES ($1, $2, gen_random_uuid(), 'delivered', $3, $4, now())",
    )
    .bind(conversation)
    .bind(sender)
    .bind(seq)
    .bind(app.state.cipher.encrypt(text))
    .execute(&app.pool)
    .await
    .unwrap();
    seq
}

async fn get(app: &TestApp, token: &str, path: &str) -> common::TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

#[sqlx::test]
async fn product_chat_is_created_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let first = product_chat(&app, &buyer, shop.product_id).await;
    let second = product_chat(&app, &buyer, shop.product_id).await;
    assert_eq!(first, second);
}

#[sqlx::test]
async fn order_chat_is_created_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let order = paid_order(&app, &buyer, &shop).await;
    let body = json!({ "order_id": order });
    let (a, b) = tokio::join!(
        open(&app, &buyer, body.clone()),
        open(&app, &shop.seller_token, body.clone())
    );
    assert_eq!(a.status, 200, "{}", a.json);
    assert_eq!(b.status, 200, "{}", b.json);
    assert_eq!(a.json["id"], b.json["id"]);
    assert_eq!(a.json["you"], "buyer");
    assert_eq!(b.json["you"], "seller");
}

#[sqlx::test]
async fn order_chat_needs_the_orders_buyer_or_store_owner(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let (_, stranger) = app.sign_up_verified(STRANGER).await;
    let order = paid_order(&app, &buyer, &shop).await;
    let res = open(&app, &stranger, json!({ "order_id": order })).await;
    assert_eq!(res.status, 404);
}

#[sqlx::test]
async fn unpaid_order_has_no_chat(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let order = paid_order(&app, &buyer, &shop).await;
    sqlx::query("UPDATE orders SET paid_at = NULL, status = 'pending_payment' WHERE id = $1")
        .bind(order)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = open(&app, &shop.seller_token, json!({ "order_id": order })).await;
    assert_eq!(res.status, 404);
}

#[sqlx::test]
async fn exactly_one_subject_is_required(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let order = paid_order(&app, &buyer, &shop).await;
    assert_eq!(open(&app, &buyer, json!({})).await.status, 422);
    let both = json!({ "product_id": shop.product_id, "order_id": order });
    assert_eq!(open(&app, &buyer, both).await.status, 422);
}

#[sqlx::test]
async fn cannot_chat_with_your_own_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let res = open(
        &app,
        &shop.seller_token,
        json!({ "product_id": shop.product_id }),
    )
    .await;
    assert_eq!(res.status, 422);
    assert_eq!(res.json["error"]["code"], "own_store_item");
}

#[sqlx::test]
async fn hidden_product_cannot_start_a_chat(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    sqlx::query("UPDATE products SET is_published = false WHERE id = $1")
        .bind(shop.product_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = open(&app, &buyer, json!({ "product_id": shop.product_id })).await;
    assert_eq!(res.status, 404);
}

#[sqlx::test]
async fn new_conversation_cap(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    for _ in 0..NEW_CONVERSATIONS_PER_DAY {
        let product = app
            .add_product(&shop.seller_token, shop.store_id, 10_000, &[(json!({}), 1)])
            .await;
        product_chat(&app, &buyer, product).await;
    }
    let res = open(&app, &buyer, json!({ "product_id": shop.product_id })).await;
    assert_eq!(res.status, 429);
    assert_eq!(res.json["error"]["code"], "chat_rate_limited");
}

#[sqlx::test]
async fn reopening_a_chat_does_not_count_against_the_cap(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    for _ in 0..NEW_CONVERSATIONS_PER_DAY + 2 {
        product_chat(&app, &buyer, shop.product_id).await;
    }
}

#[sqlx::test]
async fn seller_sees_buyer_label_not_name(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    let chat = product_chat(&app, &buyer, shop.product_id).await;
    delivered(&app, chat, buyer_id, "is it available?").await;

    let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(buyer_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let list = get(&app, &shop.seller_token, "/chats").await;
    let messages = get(&app, &shop.seller_token, &format!("/chats/{chat}/messages")).await;
    assert_eq!(list.json["conversations"][0]["with"], "Buyer");
    assert_eq!(messages.json["messages"][0]["from"], "buyer");
    for body in [list.json.to_string(), messages.json.to_string()] {
        assert!(!body.contains(&name), "{body}");
        assert!(!body.contains(&buyer_id.to_string()), "{body}");
        assert!(!body.contains(BUYER), "{body}");
    }
}

#[sqlx::test]
async fn buyer_sees_store_name_not_seller_name(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let chat = product_chat(&app, &buyer, shop.product_id).await;
    delivered(&app, chat, shop.seller_id, "yes it is").await;

    let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(shop.seller_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let list = get(&app, &buyer, "/chats").await;
    assert_eq!(list.json["conversations"][0]["with"], "Kano Fabrics");
    assert!(!list.json.to_string().contains(&name));
    assert!(!list.json.to_string().contains(&shop.seller_id.to_string()));
}

#[sqlx::test]
async fn list_holds_no_message_text(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    let chat = product_chat(&app, &buyer, shop.product_id).await;
    delivered(&app, chat, buyer_id, "a private question").await;
    let list = get(&app, &shop.seller_token, "/chats").await;
    assert!(!list.json.to_string().contains("a private question"));
}

#[sqlx::test]
async fn seller_with_two_stores_sees_both(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let second = second_store(&app, &shop).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    product_chat(&app, &buyer, shop.product_id).await;
    product_chat(&app, &buyer, second).await;
    let list = get(&app, &shop.seller_token, "/chats").await;
    let rows = list.json["conversations"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let stores: Vec<&str> = rows
        .iter()
        .map(|c| c["store_name"].as_str().unwrap())
        .collect();
    assert!(stores.contains(&"Kano Fabrics") && stores.contains(&"Kano Shoes"));
}

#[sqlx::test]
async fn staff_have_no_chat_access(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let chat = product_chat(&app, &buyer, shop.product_id).await;
    let path = format!("/chats/{chat}/messages");
    let owner = app.owner_token().await;
    let support = app.make_support().await;
    let (_, stranger) = app.sign_up_verified(STRANGER).await;
    for token in [&owner, &support, &stranger] {
        assert_eq!(get(&app, token, &path).await.status, 404);
    }
}

#[sqlx::test]
async fn read_mark_only_moves_forward(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    let chat = product_chat(&app, &buyer, shop.product_id).await;
    for _ in 0..3 {
        delivered(&app, chat, buyer_id, "hello").await;
    }
    let read = |seq: i64| {
        app.request(
            Method::POST,
            Box::leak(format!("/chats/{chat}/read").into_boxed_str()),
            Some(json!({ "seq": seq })),
            Some(&shop.seller_token),
        )
    };
    assert_eq!(read(2).await.status, 204);
    assert_eq!(read(1).await.status, 204);
    let list = get(&app, &shop.seller_token, "/chats").await;
    assert_eq!(list.json["conversations"][0]["unread"], 1);
    // A mark past the end stops at the end.
    assert_eq!(read(99).await.status, 204);
    let seq: i64 = sqlx::query_scalar("SELECT seller_read_seq FROM conversations WHERE id = $1")
        .bind(chat)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(seq, 3);
}

#[sqlx::test]
async fn unread_counts_the_other_sides_messages(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    let chat = product_chat(&app, &buyer, shop.product_id).await;
    delivered(&app, chat, buyer_id, "one").await;
    delivered(&app, chat, buyer_id, "two").await;
    delivered(&app, chat, shop.seller_id, "three").await;
    let seller = get(&app, &shop.seller_token, "/chats").await;
    assert_eq!(seller.json["conversations"][0]["unread"], 2);
    let buyer_list = get(&app, &buyer, "/chats").await;
    assert_eq!(buyer_list.json["conversations"][0]["unread"], 1);
}

#[sqlx::test]
async fn closed_store_is_read_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    let chat = product_chat(&app, &buyer, shop.product_id).await;
    delivered(&app, chat, buyer_id, "hello").await;
    sqlx::query("UPDATE stores SET status = 'suspended' WHERE id = $1")
        .bind(shop.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let list = get(&app, &buyer, "/chats").await;
    assert_eq!(list.json["conversations"][0]["writable"], false);
    let messages = get(&app, &buyer, &format!("/chats/{chat}/messages")).await;
    assert_eq!(messages.status, 200);
    assert_eq!(messages.json["messages"][0]["body"], "hello");
}

#[sqlx::test]
async fn sender_alone_sees_their_withheld_row(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    let chat = product_chat(&app, &buyer, shop.product_id).await;
    let flag: Uuid = sqlx::query_scalar(
        "INSERT INTO moderation_flags (user_id, surface, conversation_id, severity, category, \
                source, original_enc, suggestion_enc) \
         VALUES ($1, 'chat', $2, 'mild', 'rude', 'tone', $3, $4) RETURNING id",
    )
    .bind(buyer_id)
    .bind(chat)
    .bind(app.state.cipher.encrypt("rude words"))
    .bind(app.state.cipher.encrypt("kinder words"))
    .fetch_one(&app.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chat_messages (conversation_id, sender_id, client_message_id, status, \
                flag_id) VALUES ($1, $2, gen_random_uuid(), 'withheld', $3)",
    )
    .bind(chat)
    .bind(buyer_id)
    .bind(flag)
    .execute(&app.pool)
    .await
    .unwrap();

    let path = format!("/chats/{chat}/messages");
    let own = get(&app, &buyer, &path).await;
    let row = &own.json["pending"][0];
    assert_eq!(row["status"], "withheld");
    assert!(row["body"].is_null());
    assert_eq!(
        row["reason"],
        "This could come across as rude. Send this version instead?"
    );
    assert_eq!(row["suggested"], "kinder words");
    assert_eq!(row["flag_id"], flag.to_string());

    // Past the hour the suggestion is gone; the reason stays.
    sqlx::query("UPDATE moderation_flags SET created_at = now() - interval '61 minutes'")
        .execute(&app.pool)
        .await
        .unwrap();
    let own = get(&app, &buyer, &path).await;
    assert!(own.json["pending"][0]["suggested"].is_null());

    let theirs = get(&app, &shop.seller_token, &path).await;
    assert_eq!(theirs.json["messages"], json!([]));
    assert_eq!(theirs.json["pending"], json!([]));
}

#[sqlx::test]
async fn messages_page_from_a_sequence_number(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    let chat = product_chat(&app, &buyer, shop.product_id).await;
    for i in 1..=5 {
        delivered(&app, chat, buyer_id, &format!("m{i}")).await;
    }
    let res = get(
        &app,
        &buyer,
        &format!("/chats/{chat}/messages?after=2&limit=2"),
    )
    .await;
    let seqs: Vec<i64> = res.json["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, vec![3, 4]);
}

#[sqlx::test]
async fn conversations_page_by_activity(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = listing(&app).await;
    let mut tokens = Vec::new();
    for i in 0..3 {
        let (_, token) = app.sign_up_verified(&format!("0802222233{i}")).await;
        product_chat(&app, &token, shop.product_id).await;
        tokens.push(token);
    }
    let first = get(&app, &shop.seller_token, "/chats?limit=2").await;
    assert_eq!(first.json["conversations"].as_array().unwrap().len(), 2);
    let next = first.json["next_cursor"].as_str().unwrap().to_string();
    let second = get(
        &app,
        &shop.seller_token,
        &format!("/chats?limit=2&before={next}"),
    )
    .await;
    assert_eq!(second.status, 200, "{}", second.json);
    assert_eq!(second.json["conversations"].as_array().unwrap().len(), 1);
    assert!(second.json["next_cursor"].is_null());
}
