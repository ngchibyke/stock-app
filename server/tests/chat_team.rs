//! Part 7e-4: the store's team answers buyers. Access by role and branch, the identity
//! gate for sending, notices to the right people, one shared read marker, and the
//! buyer never being told which member wrote.

mod common;

use std::time::Duration;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use server::chat::hub::Subscription;
use sqlx::PgPool;
use uuid::Uuid;

struct Team {
    app: TestApp,
    store: Uuid,
    owner_token: String,
    buyer_id: Uuid,
    buyer: String,
    branch_a: Uuid,
    branch_b: Uuid,
    order: Uuid,
    product: Uuid,
    // (id, token)
    manager: (Uuid, String),
    wide: (Uuid, String),
    staff_a: (Uuid, String),
    staff_b: (Uuid, String),
    owner: Uuid,
}

async fn member(
    app: &TestApp,
    store: Uuid,
    phone: &str,
    role: &str,
    branch: Option<Uuid>,
    by: Uuid,
) -> (Uuid, String) {
    let (id, token) = app.sign_up_cleared(phone).await;
    sqlx::query(
        "INSERT INTO store_members (store_id, user_id, role, branch_id, added_by) \
         VALUES ($1, $2, $3::store_member_role, $4, $5)",
    )
    .bind(store)
    .bind(id)
    .bind(role)
    .bind(branch)
    .bind(by)
    .execute(&app.pool)
    .await
    .unwrap();
    (id, token)
}

async fn team(pool: PgPool) -> Team {
    let app = TestApp::new(pool).await;
    let f = app.shipped_order("self", "picked_up").await;
    let branch_a: Uuid = sqlx::query_scalar("SELECT branch_id FROM orders WHERE id = $1")
        .bind(f.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let branch_b: Uuid = sqlx::query_scalar(
        "INSERT INTO store_branches (store_id, name, pickup_address_enc, position) \
         SELECT store_id, 'Branch B', pickup_address_enc, 1 FROM store_branches WHERE id = $1 \
         RETURNING id",
    )
    .bind(branch_a)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let product: Uuid = sqlx::query_scalar("SELECT id FROM products WHERE store_id = $1")
        .bind(f.store_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let owner = f.seller_id;
    let manager = member(&app, f.store_id, "08066000001", "manager", None, owner).await;
    let wide = member(&app, f.store_id, "08066000002", "staff", None, owner).await;
    let staff_a = member(
        &app,
        f.store_id,
        "08066000003",
        "staff",
        Some(branch_a),
        owner,
    )
    .await;
    let staff_b = member(
        &app,
        f.store_id,
        "08066000004",
        "staff",
        Some(branch_b),
        owner,
    )
    .await;
    Team {
        app,
        store: f.store_id,
        owner_token: f.seller_token,
        buyer_id: f.buyer_id,
        buyer: f.buyer_token,
        branch_a,
        branch_b,
        order: f.order_id,
        product,
        manager,
        wide,
        staff_a,
        staff_b,
        owner,
    }
}

impl Team {
    async fn open(&self, subject: Value, token: &str) -> TestResponse {
        self.app
            .request(Method::POST, "/chats", Some(subject), Some(token))
            .await
    }

    async fn order_chat(&self) -> Uuid {
        let res = self
            .open(json!({ "order_id": self.order }), &self.buyer)
            .await;
        assert_eq!(res.status, 200, "{}", res.json);
        res.json["id"].as_str().unwrap().parse().unwrap()
    }

    async fn product_chat(&self) -> Uuid {
        let res = self
            .open(json!({ "product_id": self.product }), &self.buyer)
            .await;
        assert_eq!(res.status, 200, "{}", res.json);
        res.json["id"].as_str().unwrap().parse().unwrap()
    }

    async fn send(&self, token: &str, chat: Uuid, body: &str) -> TestResponse {
        self.app
            .request(
                Method::POST,
                &format!("/chats/{chat}/messages"),
                Some(json!({ "client_message_id": Uuid::new_v4(), "body": body })),
                Some(token),
            )
            .await
    }

    async fn messages(&self, token: &str, chat: Uuid) -> TestResponse {
        self.app
            .request(
                Method::GET,
                &format!("/chats/{chat}/messages"),
                None,
                Some(token),
            )
            .await
    }

    async fn list(&self, token: &str) -> Vec<Value> {
        let res = self
            .app
            .request(Method::GET, "/chats", None, Some(token))
            .await;
        assert_eq!(res.status, 200, "{}", res.json);
        res.json["conversations"].as_array().unwrap().clone()
    }

    async fn notices(&self, user: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM notifications WHERE user_id = $1 AND kind = 'chat_message'",
        )
        .bind(user)
        .fetch_one(&self.app.pool)
        .await
        .unwrap()
    }
}

fn ids(list: &[Value]) -> Vec<String> {
    list.iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect()
}

async fn next_frame(sub: &mut Subscription) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(2), sub.recv())
        .await
        .expect("no frame was pushed")
        .expect("the socket ended");
    serde_json::to_value(frame).unwrap()
}

#[sqlx::test]
async fn a_member_reads_and_answers_a_product_chat_and_the_buyer_never_sees_who(pool: PgPool) {
    let t = team(pool).await;
    let chat = t.product_chat().await;
    assert_eq!(t.send(&t.buyer, chat, "Is it in stock?").await.status, 200);

    for (_, token) in [&t.manager, &t.wide, &t.staff_a, &t.staff_b] {
        let list = t.list(token).await;
        assert_eq!(ids(&list), [chat.to_string()]);
        assert_eq!(list[0]["you"], "seller");
        assert_eq!(list[0]["with"], "Buyer");
        assert_eq!(list[0]["unread"], 1);
        assert_eq!(list[0]["store_id"], t.store.to_string());
    }

    // The manager answers: the team sees who, the buyer does not.
    let res = t.send(&t.manager.1, chat, "Yes, three left").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["outcome"], "delivered");
    assert_eq!(res.json["message"]["from"], "seller");
    assert!(res.json["message"]["by"].is_string());
    let seen = t.messages(&t.wide.1, chat).await;
    let reply = &seen.json["messages"][1];
    assert_eq!(reply["from"], "seller");
    assert!(reply["by"].is_string(), "{reply}");
    assert!(
        seen.json["messages"][0].get("by").is_none(),
        "the buyer's own message has no author"
    );
    let theirs = t.messages(&t.buyer, chat).await;
    assert_eq!(theirs.json["messages"][1]["from"], "seller");
    assert!(
        theirs.json["messages"][1].get("by").is_none(),
        "{theirs}",
        theirs = theirs.json
    );
    assert!(!theirs.json.to_string().contains("by"));
}

#[sqlx::test]
async fn branch_staff_see_their_branch_order_chats_and_all_product_chats(pool: PgPool) {
    let t = team(pool).await;
    let order = t.order_chat().await;
    let product = t.product_chat().await;
    assert_eq!(
        t.send(&t.buyer, order, "Where is my order?").await.status,
        200
    );
    let want = |ids: Vec<String>| {
        let mut ids = ids;
        ids.sort();
        ids
    };
    let mut both = vec![order.to_string(), product.to_string()];
    both.sort();
    for token in [&t.owner_token, &t.manager.1, &t.wide.1, &t.staff_a.1] {
        assert_eq!(want(ids(&t.list(token).await)), both);
    }
    // Branch B's staff never see branch A's order chat, but do see product chats.
    assert_eq!(ids(&t.list(&t.staff_b.1).await), [product.to_string()]);
    assert_eq!(t.messages(&t.staff_b.1, order).await.status, 404);
    assert_eq!(t.send(&t.staff_b.1, order, "hi").await.status, 404);
    assert_eq!(t.messages(&t.staff_a.1, order).await.status, 200);
    // The order chat names its branch to the store's side only.
    let a = t.list(&t.staff_a.1).await;
    let row = a.iter().find(|c| c["id"] == order.to_string()).unwrap();
    assert!(row["branch_name"].is_string());
    let buyer = t.list(&t.buyer).await;
    assert!(buyer.iter().all(|c| c.get("branch_name").is_none()));
}

#[sqlx::test]
async fn strangers_removed_members_and_closed_branches_lose_access(pool: PgPool) {
    let t = team(pool).await;
    let order = t.order_chat().await;
    let product = t.product_chat().await;
    let (_, stranger) = t.app.make_seller("08066000099").await;
    let support = t.app.make_support().await;
    for token in [&stranger, &support] {
        assert_eq!(t.messages(token, order).await.status, 404);
        assert!(t.list(token).await.is_empty());
    }
    // A removed member is out on their next request.
    sqlx::query("UPDATE store_members SET removed_at = now(), removed_by = $1 WHERE user_id = $2")
        .bind(t.owner)
        .bind(t.wide.0)
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(t.messages(&t.wide.1, product).await.status, 404);
    // Staff tied to a branch that closed keep product chats but lose order chats.
    sqlx::query("UPDATE store_branches SET closed_at = now() WHERE id = $1")
        .bind(t.branch_b)
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(t.messages(&t.staff_b.1, product).await.status, 200);
    sqlx::query("UPDATE store_branches SET closed_at = now() WHERE id = $1")
        .bind(t.branch_a)
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(t.messages(&t.staff_a.1, order).await.status, 404);
}

#[sqlx::test]
async fn a_member_must_pass_the_identity_check_to_send_but_may_read(pool: PgPool) {
    let t = team(pool).await;
    let chat = t.product_chat().await;
    assert_eq!(t.send(&t.buyer, chat, "Hello").await.status, 200);
    sqlx::query("DELETE FROM identity_checks WHERE user_id = $1")
        .bind(t.manager.0)
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(t.messages(&t.manager.1, chat).await.status, 200);
    let res = t.send(&t.manager.1, chat, "Hi there").await;
    assert_eq!(res.status, 403, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "identity_required");
    // The owner needs no check of their own to answer.
    sqlx::query("DELETE FROM identity_checks WHERE user_id = $1")
        .bind(t.owner)
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(t.send(&t.owner_token, chat, "Welcome").await.status, 200);
    t.app.pass_identity(t.manager.0).await;
    assert_eq!(t.send(&t.manager.1, chat, "Hi there").await.status, 200);
}

#[sqlx::test]
async fn a_buyers_message_notifies_the_team_by_role_and_branch(pool: PgPool) {
    let t = team(pool).await;
    let order = t.order_chat().await;
    assert_eq!(
        t.send(&t.buyer, order, "Question about my order")
            .await
            .status,
        200
    );
    for user in [t.owner, t.manager.0, t.wide.0, t.staff_a.0] {
        assert_eq!(t.notices(user).await, 1, "{user}");
    }
    assert_eq!(
        t.notices(t.staff_b.0).await,
        0,
        "the other branch is not told"
    );
    // A second message before anyone reads shares the notice.
    assert_eq!(t.send(&t.buyer, order, "Hello?").await.status, 200);
    assert_eq!(t.notices(t.manager.0).await, 1);
    // A product chat tells the owner, managers and store-wide staff, and both branches' staff.
    let product = t.product_chat().await;
    assert_eq!(t.send(&t.buyer, product, "Is it new?").await.status, 200);
    assert_eq!(t.notices(t.staff_b.0).await, 1);
    // The store's answer tells the buyer only.
    let before = t.notices(t.buyer_id).await;
    assert_eq!(t.send(&t.manager.1, order, "On its way").await.status, 200);
    assert_eq!(t.notices(t.buyer_id).await, before + 1);
    assert_eq!(
        t.notices(t.manager.0).await,
        2,
        "no notice for their own message"
    );
}

async fn register_device(t: &Team, token: &str, push_token: &str) {
    let res = t
        .app
        .request(
            Method::PUT,
            "/me/push-device",
            Some(json!({ "token": push_token, "platform": "android" })),
            Some(token),
        )
        .await;
    assert!(res.status < 300, "{}", res.json);
}

/// Push tokens that received a chat notice since the last call.
fn chat_pushes(t: &Team) -> Vec<(String, String, String)> {
    t.app
        .push
        .seen()
        .into_iter()
        .filter(|s| s.kind == "chat_message")
        .map(|s| (s.token, s.title.to_string(), s.body.to_string()))
        .collect()
}

#[sqlx::test]
async fn a_chat_message_reaches_the_right_devices_with_fixed_words_only(pool: PgPool) {
    let t = team(pool).await;
    let order = t.order_chat().await;
    register_device(&t, &t.owner_token, "dev-owner").await;
    register_device(&t, &t.staff_a.1, "dev-staff-a").await;
    register_device(&t, &t.staff_b.1, "dev-staff-b").await;
    register_device(&t, &t.buyer, "dev-buyer").await;

    let text = "Please confirm the size of my dress";
    assert_eq!(t.send(&t.buyer, order, text).await.status, 200);
    server::notify::worker::run_once(&t.app.state)
        .await
        .unwrap();
    let mut tokens: Vec<String> = chat_pushes(&t).into_iter().map(|p| p.0).collect();
    tokens.sort();
    assert_eq!(
        tokens,
        ["dev-owner", "dev-staff-a"],
        "the order's team, not the other branch, not the sender"
    );
    for (_, title, body) in chat_pushes(&t) {
        assert_eq!(title, "New message");
        assert_eq!(body, "You have a new message. Open the app to read it.");
        assert!(!body.contains("dress") && !title.contains("dress"));
    }

    // The store's answer reaches the buyer's device only.
    let before = chat_pushes(&t).len();
    assert_eq!(t.send(&t.manager.1, order, "Size M").await.status, 200);
    server::notify::worker::run_once(&t.app.state)
        .await
        .unwrap();
    let new: Vec<_> = chat_pushes(&t).split_off(before);
    assert_eq!(new.len(), 1);
    assert_eq!(new[0].0, "dev-buyer");
}

#[sqlx::test]
async fn the_team_shares_one_read_marker_and_answers_are_not_unread_for_each_other(pool: PgPool) {
    let t = team(pool).await;
    let chat = t.product_chat().await;
    assert_eq!(t.send(&t.buyer, chat, "Hello").await.status, 200);
    assert_eq!(t.list(&t.wide.1).await[0]["unread"], 1);
    // One member reads: the rest see it read.
    let seq = t.messages(&t.manager.1, chat).await.json["messages"][0]["seq"]
        .as_i64()
        .unwrap();
    let res = t
        .app
        .request(
            Method::POST,
            &format!("/chats/{chat}/read"),
            Some(json!({ "seq": seq })),
            Some(&t.manager.1),
        )
        .await;
    assert_eq!(res.status, 204);
    for token in [&t.wide.1, &t.owner_token, &t.staff_a.1] {
        assert_eq!(t.list(token).await[0]["unread"], 0);
    }
    // A teammate's own reply does not count as unread for another teammate.
    assert_eq!(t.send(&t.manager.1, chat, "On it").await.status, 200);
    assert_eq!(t.list(&t.wide.1).await[0]["unread"], 0);
    // The buyer has the store's reply unread.
    assert_eq!(t.list(&t.buyer).await[0]["unread"], 1);
}

#[sqlx::test]
async fn a_member_cannot_open_a_product_chat_about_their_own_store_and_may_open_a_branch_order_chat(
    pool: PgPool,
) {
    let t = team(pool).await;
    let res = t
        .open(json!({ "product_id": t.product }), &t.manager.1)
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "own_store_item");
    // An order's chat can be opened by the store's side that covers its branch.
    assert_eq!(
        t.open(json!({ "order_id": t.order }), &t.staff_a.1)
            .await
            .status,
        200
    );
    assert_eq!(
        t.open(json!({ "order_id": t.order }), &t.staff_b.1)
            .await
            .status,
        404
    );
}

#[sqlx::test]
async fn frames_reach_each_person_who_may_see_the_chat_and_hide_the_author_from_the_buyer(
    pool: PgPool,
) {
    let t = team(pool).await;
    let order = t.order_chat().await;
    let mut buyer = t.app.state.chat_hub.subscribe(t.buyer_id).unwrap();
    let mut manager = t.app.state.chat_hub.subscribe(t.manager.0).unwrap();
    let mut staff_b = t.app.state.chat_hub.subscribe(t.staff_b.0).unwrap();
    assert_eq!(t.send(&t.wide.1, order, "We have it").await.status, 200);
    let to_buyer = next_frame(&mut buyer).await;
    let to_manager = next_frame(&mut manager).await;
    assert_eq!(to_buyer["type"], "message");
    assert!(to_buyer.get("by").is_none(), "{to_buyer}");
    assert!(to_manager["by"].is_string(), "{to_manager}");
    // Branch B's staff are not told about branch A's order chat.
    assert!(
        tokio::time::timeout(Duration::from_millis(300), staff_b.recv())
            .await
            .is_err()
    );
}
