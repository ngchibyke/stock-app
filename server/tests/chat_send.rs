//! Sending: every message passes moderation before the other side can see it.

mod common;

use std::time::Duration;

use axum::http::Method;
use common::{Listing, TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    chat::{held, hub::Subscription, send::apply_verdict},
    limits::{CHAT_SENDS_PER_MINUTE, MAX_CHAT_CHARS},
    moderation::{
        filter::Category,
        pipeline::Verdict,
        tone::{ToneError, ToneVerdict},
    },
};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing_test::traced_test;
use uuid::Uuid;

const SELLER: &str = "08055555551";
const BUYER: &str = "08022222221";
const PHONE_TEXT: &str = "call me on 08031234567";
const CONTACT_COPY: &str =
    "Keep contact details and payments inside the app. This message wasn't sent.";

struct Chat {
    app: TestApp,
    shop: Listing,
    buyer_id: Uuid,
    buyer: String,
    id: Uuid,
}

async fn chat(pool: PgPool) -> Chat {
    let app = TestApp::new(pool).await;
    let shop = app
        .make_active_store_with_product(SELLER, "Kano Fabrics", 10_000, 5)
        .await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    let res = app
        .request(
            Method::POST,
            "/chats",
            Some(json!({ "product_id": shop.product_id })),
            Some(&buyer),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let id = res.json["id"].as_str().unwrap().parse().unwrap();
    Chat {
        app,
        shop,
        buyer_id,
        buyer,
        id,
    }
}

impl Chat {
    async fn send_as(&self, token: &str, client: Uuid, body: &str) -> TestResponse {
        self.app
            .request(
                Method::POST,
                &format!("/chats/{}/messages", self.id),
                Some(json!({ "client_message_id": client, "body": body })),
                Some(token),
            )
            .await
    }

    async fn send(&self, body: &str) -> TestResponse {
        self.send_as(&self.buyer, Uuid::new_v4(), body).await
    }

    async fn use_suggestion(&self, token: &str, flag: &Value, body: Value) -> TestResponse {
        self.app
            .request(
                Method::POST,
                &format!(
                    "/chats/{}/suggestions/{}/send",
                    self.id,
                    flag.as_str().unwrap()
                ),
                Some(body),
                Some(token),
            )
            .await
    }

    async fn list(&self, token: &str) -> Value {
        let res = self
            .app
            .request(
                Method::GET,
                &format!("/chats/{}/messages", self.id),
                None,
                Some(token),
            )
            .await;
        assert_eq!(res.status, 200, "{}", res.json);
        res.json
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }

    async fn strikes(&self) -> i64 {
        self.count("SELECT count(*) FROM moderation_strikes").await
    }

    async fn rows(&self) -> i64 {
        self.count("SELECT count(*) FROM chat_messages").await
    }
}

/// The next pushed frame. A push that never comes fails rather than hangs.
async fn next_frame(sub: &mut Subscription) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(2), sub.recv())
        .await
        .expect("no frame was pushed")
        .expect("the socket ended");
    serde_json::to_value(frame).unwrap()
}

fn bodies(list: &Value) -> Vec<String> {
    list["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test]
async fn clean_message_reaches_the_other_side_with_a_seq(pool: PgPool) {
    let c = chat(pool).await;
    let res = c.send("  is it available?  ").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["outcome"], "delivered");
    assert_eq!(res.json["message"]["seq"], 1);
    let theirs = c.list(&c.shop.seller_token).await;
    assert_eq!(theirs["messages"][0]["body"], "is it available?");
    assert_eq!(theirs["messages"][0]["from"], "buyer");
    assert_eq!(theirs["messages"][0]["seq"], 1);

    let reply = c
        .send_as(&c.shop.seller_token, Uuid::new_v4(), "yes it is")
        .await;
    assert_eq!(reply.json["message"]["seq"], 2);
    assert_eq!(reply.json["message"]["from"], "seller");
    let last: bool = sqlx::query_scalar(
        "SELECT last_seq = 2 AND last_message_at > created_at FROM conversations WHERE id = $1",
    )
    .bind(c.id)
    .fetch_one(&c.app.pool)
    .await
    .unwrap();
    assert!(last);
}

#[sqlx::test]
async fn message_is_pushed_to_both_parties(pool: PgPool) {
    let c = chat(pool).await;
    let mut seller = c.app.state.chat_hub.subscribe(c.shop.seller_id).unwrap();
    let mut buyer = c.app.state.chat_hub.subscribe(c.buyer_id).unwrap();
    c.send("hello").await;
    let frame = next_frame(&mut seller).await;
    assert_eq!(frame["type"], "message");
    assert_eq!(frame["body"], "hello");
    next_frame(&mut buyer).await;
}

#[sqlx::test]
async fn blocked_sender_alone_is_told(pool: PgPool) {
    let c = chat(pool).await;
    let mut seller = c.app.state.chat_hub.subscribe(c.shop.seller_id).unwrap();
    let mut buyer = c.app.state.chat_hub.subscribe(c.buyer_id).unwrap();
    c.send(PHONE_TEXT).await;
    let frame = next_frame(&mut buyer).await;
    assert_eq!(frame["type"], "moderation");
    assert_eq!(frame["outcome"]["outcome"], "blocked");
    c.send("hello").await;
    let frame = next_frame(&mut seller).await;
    assert_eq!(frame["type"], "message");
    assert_eq!(frame["body"], "hello", "the seller saw the blocked message");
}

#[sqlx::test]
async fn recipient_never_sees_flagged_text(pool: PgPool) {
    let c = chat(pool).await;
    assert_eq!(c.send(PHONE_TEXT).await.json["outcome"], "blocked");
    c.app
        .tone
        .set_next(Ok(ToneVerdict::mild("Please check this.")));
    assert_eq!(c.send("you fool").await.json["outcome"], "rewrite");
    assert_eq!(c.send("thank you").await.json["message"]["seq"], 1);

    let theirs = c.list(&c.shop.seller_token).await;
    assert_eq!(bodies(&theirs), vec!["thank you"]);
    assert_eq!(theirs["pending"], json!([]));
    assert!(!theirs.to_string().contains("08031234567"));
    assert!(!theirs.to_string().contains("fool"));

    let mine = c.list(&c.buyer).await;
    let pending = mine["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 2);
    assert!(pending.iter().all(|p| p["status"] == "withheld"));
    assert!(pending.iter().all(|p| p["body"].is_null()));
}

#[sqlx::test]
async fn blocked_message_warns_and_strikes(pool: PgPool) {
    let c = chat(pool).await;
    let res = c.send(PHONE_TEXT).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json["outcome"], "blocked");
    assert_eq!(res.json["reason"], CONTACT_COPY);
    assert_eq!(res.json["strikes"], 1);
    assert!(res.json.get("message").is_none());
    assert_eq!(c.strikes().await, 1);
    assert_eq!(c.app.tone.calls(), 0);
}

#[sqlx::test]
async fn mild_message_offers_a_suggestion(pool: PgPool) {
    let c = chat(pool).await;
    c.app
        .tone
        .set_next(Ok(ToneVerdict::mild("Please check this.")));
    let res = c.send("sort this out now").await;
    assert_eq!(res.json["outcome"], "rewrite");
    assert_eq!(res.json["suggested"], "Please check this.");
    assert_eq!(
        res.json["reason"],
        "This could come across as rude. Send this version instead?"
    );
    assert!(res.json["flag_id"].is_string());
    assert_eq!(c.strikes().await, 0);
}

#[sqlx::test]
async fn send_suggested_uses_server_text_once(pool: PgPool) {
    let c = chat(pool).await;
    c.app
        .tone
        .set_next(Ok(ToneVerdict::mild("Please check this.")));
    let res = c.send("sort this out now").await;
    let flag = res.json["flag_id"].clone();

    let forged = json!({ "client_message_id": Uuid::new_v4(), "body": "something else" });
    let res = c.use_suggestion(&c.buyer, &flag, forged).await;
    assert_eq!(res.status, 422, "a client body must be refused");

    let stranger = c
        .use_suggestion(
            &c.shop.seller_token,
            &flag,
            json!({ "client_message_id": Uuid::new_v4() }),
        )
        .await;
    assert_eq!(stranger.status, 404, "another user's flag");

    let used = json!({ "client_message_id": Uuid::new_v4() });
    let res = c.use_suggestion(&c.buyer, &flag, used.clone()).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["outcome"], "delivered");
    assert_eq!(res.json["message"]["body"], "Please check this.");
    assert_eq!(
        bodies(&c.list(&c.shop.seller_token).await),
        vec!["Please check this."]
    );

    // The same client id is a retry, and answers as before.
    let retry = c.use_suggestion(&c.buyer, &flag, used).await;
    assert_eq!(retry.status, 200);
    assert_eq!(retry.json["message"]["seq"], 1);

    let again = json!({ "client_message_id": Uuid::new_v4() });
    let res = c.use_suggestion(&c.buyer, &flag, again).await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "suggestion_used");
    assert_eq!(c.app.tone.calls(), 1, "the suggestion is not judged again");
}

#[sqlx::test]
async fn expired_suggestion_is_refused(pool: PgPool) {
    let c = chat(pool).await;
    c.app
        .tone
        .set_next(Ok(ToneVerdict::mild("Please check this.")));
    let res = c.send("sort this out now").await;
    let flag = res.json["flag_id"].clone();
    sqlx::query("UPDATE moderation_flags SET created_at = now() - interval '61 minutes'")
        .execute(&c.app.pool)
        .await
        .unwrap();
    let res = c
        .use_suggestion(
            &c.buyer,
            &flag,
            json!({ "client_message_id": Uuid::new_v4() }),
        )
        .await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "suggestion_expired");
}

#[sqlx::test]
async fn edited_suggestion_goes_through_the_full_pipeline(pool: PgPool) {
    let c = chat(pool).await;
    c.app
        .tone
        .set_next(Ok(ToneVerdict::mild("Please check this.")));
    c.send("sort this out now").await;
    let res = c.send("Please check this, 08031234567").await;
    assert_eq!(res.json["outcome"], "blocked");
    let res = c.send("Please check this, thanks").await;
    assert_eq!(res.json["outcome"], "delivered");
    assert_eq!(c.app.tone.calls(), 2);
}

#[sqlx::test]
async fn retry_returns_first_outcome(pool: PgPool) {
    let c = chat(pool).await;
    let delivered = Uuid::new_v4();
    let first = c.send_as(&c.buyer, delivered, "hello").await;
    let second = c.send_as(&c.buyer, delivered, "hello").await;
    assert_eq!(first.json, second.json);

    let blocked = Uuid::new_v4();
    let first = c.send_as(&c.buyer, blocked, PHONE_TEXT).await;
    let second = c.send_as(&c.buyer, blocked, PHONE_TEXT).await;
    assert_eq!(first.json, second.json);
    assert_eq!(c.strikes().await, 1);

    let rewrite = Uuid::new_v4();
    c.app
        .tone
        .set_next(Ok(ToneVerdict::mild("Please check this.")));
    let first = c.send_as(&c.buyer, rewrite, "sort this out").await;
    let second = c.send_as(&c.buyer, rewrite, "sort this out").await;
    assert_eq!(first.json, second.json);

    assert_eq!(c.rows().await, 3);
    assert_eq!(c.app.tone.calls(), 2);
}

#[sqlx::test]
async fn concurrent_identical_sends_make_one_row(pool: PgPool) {
    let c = chat(pool).await;
    let client = Uuid::new_v4();
    let (a, b) = tokio::join!(
        c.send_as(&c.buyer, client, PHONE_TEXT),
        c.send_as(&c.buyer, client, PHONE_TEXT)
    );
    assert_eq!(a.json, b.json);
    assert_eq!(c.rows().await, 1);
    assert_eq!(c.strikes().await, 1);
    assert_eq!(c.count("SELECT count(*) FROM moderation_flags").await, 1);
}

#[sqlx::test]
async fn client_id_cannot_cross_conversations(pool: PgPool) {
    let c = chat(pool).await;
    let client = Uuid::new_v4();
    c.send_as(&c.buyer, client, "hello").await;
    let product = c
        .app
        .add_product(
            &c.shop.seller_token,
            c.shop.store_id,
            10_000,
            &[(json!({}), 1)],
        )
        .await;
    let res = c
        .app
        .request(
            Method::POST,
            "/chats",
            Some(json!({ "product_id": product })),
            Some(&c.buyer),
        )
        .await;
    let second = res.json["id"].as_str().unwrap();
    let res = c
        .app
        .request(
            Method::POST,
            &format!("/chats/{second}/messages"),
            Some(json!({ "client_message_id": client, "body": "hello" })),
            Some(&c.buyer),
        )
        .await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "client_message_id_reused");
}

#[sqlx::test]
async fn phone_split_across_messages_is_blocked(pool: PgPool) {
    let c = chat(pool).await;
    assert_eq!(c.send("0803").await.json["outcome"], "delivered");
    assert_eq!(c.send("123").await.json["outcome"], "delivered");
    assert_eq!(c.send("4567").await.json["outcome"], "blocked");
}

#[sqlx::test]
async fn split_digits_do_not_join_across_senders(pool: PgPool) {
    let c = chat(pool).await;
    c.send("0803").await;
    let seller = c.shop.seller_token.clone();
    c.send_as(&seller, Uuid::new_v4(), "123").await;
    let res = c.send("4567").await;
    assert_eq!(res.json["outcome"], "delivered", "{}", res.json);
}

#[sqlx::test]
async fn muted_sender_gets_403_with_muted_until(pool: PgPool) {
    let c = chat(pool).await;
    sqlx::query(
        "INSERT INTO user_moderation (user_id, muted_until) \
         VALUES ($1, now() + interval '1 hour')",
    )
    .bind(c.buyer_id)
    .execute(&c.app.pool)
    .await
    .unwrap();
    let res = c.send("hello").await;
    assert_eq!(res.status, 403);
    assert_eq!(res.json["error"]["code"], "muted");
    assert!(res.json["muted_until"].is_string());
    let copy = res.json["error"]["message"].as_str().unwrap();
    assert!(copy.starts_with("You can't post until "), "{copy}");
    assert!(
        copy.ends_with(", because of repeated rule breaks."),
        "{copy}"
    );
    assert_eq!(c.rows().await, 0);
}

#[sqlx::test]
async fn third_strike_comes_back_with_the_mute(pool: PgPool) {
    let c = chat(pool).await;
    for _ in 0..3 {
        let res = c.send(PHONE_TEXT).await;
        assert_eq!(res.json["outcome"], "blocked", "{}", res.json);
        sqlx::query("UPDATE moderation_strikes SET at = at - interval '11 minutes'")
            .execute(&c.app.pool)
            .await
            .unwrap();
        if res.json["strikes"] == 3 {
            assert!(res.json["muted_until"].is_string());
            return;
        }
    }
    panic!("no mute");
}

#[sqlx::test]
async fn banned_sender_is_refused(pool: PgPool) {
    let c = chat(pool).await;
    sqlx::query("INSERT INTO user_moderation (user_id, banned_at) VALUES ($1, now())")
        .bind(c.buyer_id)
        .execute(&c.app.pool)
        .await
        .unwrap();
    let res = c.send("hello").await;
    assert_eq!(res.status, 403);
    assert_eq!(res.json["error"]["code"], "posting_banned");
}

#[sqlx::test]
async fn closed_chat_refuses_sends(pool: PgPool) {
    let c = chat(pool).await;
    sqlx::query("UPDATE stores SET status = 'suspended' WHERE id = $1")
        .bind(c.shop.store_id)
        .execute(&c.app.pool)
        .await
        .unwrap();
    let res = c.send("hello").await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "chat_closed");
}

#[sqlx::test]
async fn rate_cap_returns_429(pool: PgPool) {
    let c = chat(pool).await;
    sqlx::query(
        "INSERT INTO chat_messages (conversation_id, sender_id, client_message_id, status) \
         SELECT $1, $2, gen_random_uuid(), 'withheld' FROM generate_series(1, $3)",
    )
    .bind(c.id)
    .bind(c.buyer_id)
    .bind(CHAT_SENDS_PER_MINUTE as i32)
    .execute(&c.app.pool)
    .await
    .unwrap();
    let res = c.send("hello").await;
    assert_eq!(res.status, 429);
    assert_eq!(res.json["error"]["code"], "chat_rate_limited");
    assert_eq!(c.app.tone.calls(), 0);
}

#[sqlx::test]
async fn daily_cap_returns_429(pool: PgPool) {
    let c = chat(pool).await;
    sqlx::query(
        "INSERT INTO chat_messages (conversation_id, sender_id, client_message_id, status, \
                created_at) \
         SELECT $1, $2, gen_random_uuid(), 'withheld', now() - interval '2 hours' \
         FROM generate_series(1, 500)",
    )
    .bind(c.id)
    .bind(c.buyer_id)
    .execute(&c.app.pool)
    .await
    .unwrap();
    assert_eq!(c.send("hello").await.status, 429);
}

#[sqlx::test]
async fn body_limit_counts_characters(pool: PgPool) {
    let c = chat(pool).await;
    let full = "é".repeat(MAX_CHAT_CHARS);
    assert_eq!(c.send(&full).await.status, 200);
    let over = "é".repeat(MAX_CHAT_CHARS + 1);
    assert_eq!(c.send(&over).await.status, 422);
    assert_eq!(c.send("   \n ").await.status, 422);
}

#[sqlx::test]
async fn outage_holds_the_message(pool: PgPool) {
    let c = chat(pool).await;
    c.app.tone.set_next(Err(ToneError::Unavailable));
    let res = c.send("hello").await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json["outcome"], "held");
    assert_eq!(c.list(&c.shop.seller_token).await["messages"], json!([]));
    let mine = c.list(&c.buyer).await;
    assert_eq!(mine["pending"][0]["status"], "held");
    assert_eq!(mine["pending"][0]["body"], "hello");
}

#[sqlx::test]
async fn nothing_overtakes_a_held_message(pool: PgPool) {
    let c = chat(pool).await;
    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("first").await;
    let calls = c.app.tone.calls();
    assert_eq!(c.send("second").await.json["outcome"], "held");
    assert_eq!(c.app.tone.calls(), calls, "no check while one is held");
    assert_eq!(c.send(PHONE_TEXT).await.json["outcome"], "blocked");
}

#[sqlx::test]
async fn tone_block_uses_the_fixed_copy(pool: PgPool) {
    let c = chat(pool).await;
    c.app
        .tone
        .set_next(Ok(ToneVerdict::serious(Category::Abuse)));
    let res = c.send("a subtle threat").await;
    assert_eq!(
        res.json["reason"],
        "This message breaks the market's rules and wasn't sent."
    );
}

#[sqlx::test]
async fn refused_text_is_blocked_with_no_strike(pool: PgPool) {
    let c = chat(pool).await;
    c.app.tone.set_next(Err(ToneError::Refused));
    let res = c.send("odd text").await;
    assert_eq!(res.json["outcome"], "blocked");
    assert_eq!(
        res.json["reason"],
        "We couldn't check this message, so it wasn't sent. Try wording it differently."
    );
    assert_eq!(c.strikes().await, 0);
}

#[sqlx::test]
async fn stored_body_is_encrypted(pool: PgPool) {
    let c = chat(pool).await;
    c.send("a very private question").await;
    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("a held private question").await;
    let rows: Vec<String> = sqlx::query_scalar("SELECT to_jsonb(m)::text FROM chat_messages m")
        .fetch_all(&c.app.pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| !r.contains("private")));
}

#[traced_test]
#[sqlx::test]
async fn no_message_text_in_logs(pool: PgPool) {
    let c = chat(pool).await;
    c.send("loggable clean text").await;
    c.send("loggable 08031234567").await;
    c.app
        .tone
        .set_next(Ok(ToneVerdict::mild("loggable suggestion")));
    c.send("loggable mild text").await;
    assert!(logs_contain("response"), "expected request logs");
    assert!(!logs_contain("loggable"));
    assert!(!logs_contain("08031234567"));
}

#[sqlx::test]
async fn audit_has_no_message_text(pool: PgPool) {
    let c = chat(pool).await;
    c.send("audited 08031234567").await;
    let details: Vec<Value> = sqlx::query_scalar("SELECT detail FROM audit_log")
        .fetch_all(&c.app.pool)
        .await
        .unwrap();
    assert!(!details.is_empty());
    assert!(details.iter().all(|d| !d.to_string().contains("audited")));
}

/// Makes held rows due, as if their lease or backoff had passed.
async fn make_due(c: &Chat) {
    sqlx::query(
        "UPDATE chat_messages SET next_attempt_at = now() - interval '1 second' \
         WHERE status = 'held'",
    )
    .execute(&c.app.pool)
    .await
    .unwrap();
}

async fn run_job(c: &Chat) {
    make_due(c).await;
    held::run(&c.app.state, &CancellationToken::new())
        .await
        .unwrap();
}

async fn hold_three(c: &Chat) {
    c.app.tone.set_all(Err(ToneError::Unavailable));
    for text in ["first", "second", "third"] {
        assert_eq!(c.send(text).await.json["outcome"], "held");
    }
}

#[sqlx::test]
async fn outage_holds_then_delivers_in_order(pool: PgPool) {
    let c = chat(pool).await;
    hold_three(&c).await;
    assert_eq!(c.list(&c.shop.seller_token).await["messages"], json!([]));
    let mine = c.list(&c.buyer).await;
    assert!(
        mine["pending"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["status"] == "held")
    );

    c.app.tone.reset();
    for _ in 0..3 {
        run_job(&c).await;
    }
    let theirs = c.list(&c.shop.seller_token).await;
    assert_eq!(bodies(&theirs), vec!["first", "second", "third"]);
    let seqs: Vec<i64> = theirs["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["seq"].as_i64().unwrap())
        .collect();
    assert_eq!(seqs, vec![1, 2, 3]);
    assert_eq!(c.list(&c.buyer).await["pending"], json!([]));
}

#[sqlx::test]
async fn a_failed_row_is_not_overtaken(pool: PgPool) {
    let c = chat(pool).await;
    hold_three(&c).await;
    // Still failing: one row is tried, and those behind it wait.
    let before = c.app.tone.calls();
    run_job(&c).await;
    assert_eq!(c.app.tone.calls(), before + 1);
    assert_eq!(c.send("fourth").await.json["outcome"], "held");
    assert_eq!(c.list(&c.shop.seller_token).await["messages"], json!([]));
}

#[sqlx::test]
async fn verdict_after_hold_can_block_or_offer_rewrite(pool: PgPool) {
    let c = chat(pool).await;
    let mut buyer = c.app.state.chat_hub.subscribe(c.buyer_id).unwrap();
    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("a veiled threat").await;
    next_frame(&mut buyer).await;

    c.app
        .tone
        .set_next(Ok(ToneVerdict::serious(Category::Abuse)));
    run_job(&c).await;
    let frame = next_frame(&mut buyer).await;
    assert_eq!(frame["type"], "moderation");
    assert_eq!(frame["outcome"]["outcome"], "blocked");
    assert_eq!(c.strikes().await, 1);

    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("sort this out").await;
    next_frame(&mut buyer).await;
    c.app
        .tone
        .set_next(Ok(ToneVerdict::mild("Please check this.")));
    run_job(&c).await;
    let frame = next_frame(&mut buyer).await;
    assert_eq!(frame["outcome"]["outcome"], "rewrite");
    assert_eq!(frame["outcome"]["suggested"], "Please check this.");
    assert_eq!(c.strikes().await, 1);
    assert_eq!(c.list(&c.shop.seller_token).await["messages"], json!([]));
}

#[sqlx::test]
async fn delivered_after_hold_tells_both_sides(pool: PgPool) {
    let c = chat(pool).await;
    let mut seller = c.app.state.chat_hub.subscribe(c.shop.seller_id).unwrap();
    let mut buyer = c.app.state.chat_hub.subscribe(c.buyer_id).unwrap();
    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("hello").await;
    next_frame(&mut buyer).await;
    run_job(&c).await;
    assert_eq!(next_frame(&mut seller).await["body"], "hello");
    let mut kinds = vec![
        next_frame(&mut buyer).await["type"].clone(),
        next_frame(&mut buyer).await["type"].clone(),
    ];
    kinds.sort_by_key(|k| k.to_string());
    assert_eq!(kinds, vec![json!("message"), json!("moderation")]);
}

#[sqlx::test]
async fn held_message_gives_up_after_six_hours(pool: PgPool) {
    let c = chat(pool).await;
    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("hello").await;
    sqlx::query("UPDATE chat_messages SET created_at = now() - interval '6 hours 1 minute'")
        .execute(&c.app.pool)
        .await
        .unwrap();
    let calls = c.app.tone.calls();
    run_job(&c).await;
    assert_eq!(c.app.tone.calls(), calls, "no check after giving up");
    let (status, cleared): (String, bool) =
        sqlx::query_as("SELECT status, body_enc IS NULL FROM chat_messages")
            .fetch_one(&c.app.pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), cleared), ("withheld", true));
    assert_eq!(c.strikes().await, 0);
    let mine = c.list(&c.buyer).await;
    assert_eq!(
        mine["pending"][0]["reason"],
        "We couldn't check this message, so it wasn't sent. Try wording it differently."
    );
}

#[sqlx::test]
async fn held_message_from_a_banned_sender_is_not_delivered(pool: PgPool) {
    let c = chat(pool).await;
    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("hello").await;
    sqlx::query("INSERT INTO user_moderation (user_id, banned_at) VALUES ($1, now())")
        .bind(c.buyer_id)
        .execute(&c.app.pool)
        .await
        .unwrap();
    run_job(&c).await;
    assert_eq!(c.list(&c.shop.seller_token).await["messages"], json!([]));
    assert_eq!(c.strikes().await, 0);
    assert_eq!(c.list(&c.buyer).await["pending"][0]["status"], "withheld");
}

#[sqlx::test]
async fn held_message_in_a_closed_chat_is_not_delivered(pool: PgPool) {
    let c = chat(pool).await;
    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("hello").await;
    sqlx::query("UPDATE stores SET status = 'suspended' WHERE id = $1")
        .bind(c.shop.store_id)
        .execute(&c.app.pool)
        .await
        .unwrap();
    run_job(&c).await;
    assert_eq!(c.list(&c.shop.seller_token).await["messages"], json!([]));
}

#[sqlx::test]
async fn two_job_runs_never_double_deliver(pool: PgPool) {
    let c = chat(pool).await;
    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("hello").await;
    make_due(&c).await;
    let cancel = CancellationToken::new();
    let (a, b) = tokio::join!(
        held::run(&c.app.state, &cancel),
        held::run(&c.app.state, &cancel)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(bodies(&c.list(&c.shop.seller_token).await), vec!["hello"]);
    let last: i64 = sqlx::query_scalar("SELECT last_seq FROM conversations WHERE id = $1")
        .bind(c.id)
        .fetch_one(&c.app.pool)
        .await
        .unwrap();
    assert_eq!(last, 1);
}

#[sqlx::test]
async fn verdict_is_ignored_once_a_row_has_moved_on(pool: PgPool) {
    let c = chat(pool).await;
    c.app.tone.set_next(Err(ToneError::Unavailable));
    c.send("hello").await;
    let id: Uuid = sqlx::query_scalar("SELECT id FROM chat_messages")
        .fetch_one(&c.app.pool)
        .await
        .unwrap();
    let first = apply_verdict(&c.app.state, id, &Verdict::Clean)
        .await
        .unwrap();
    let second = apply_verdict(&c.app.state, id, &Verdict::Clean)
        .await
        .unwrap();
    assert_eq!(first.outcome, "delivered");
    assert_eq!(second.outcome, "delivered");
    assert_eq!(bodies(&c.list(&c.shop.seller_token).await), vec!["hello"]);
}

#[sqlx::test]
async fn held_rows_stay_encrypted(pool: PgPool) {
    let c = chat(pool).await;
    hold_three(&c).await;
    run_job(&c).await;
    let rows: Vec<String> = sqlx::query_scalar("SELECT to_jsonb(m)::text FROM chat_messages m")
        .fetch_all(&c.app.pool)
        .await
        .unwrap();
    assert!(rows.iter().all(|r| !r.contains("first")));
}

#[sqlx::test]
async fn tone_failures_back_off(pool: PgPool) {
    let c = chat(pool).await;
    c.app.tone.set_all(Err(ToneError::Unavailable));
    c.send("hello").await;
    let due_in = || async {
        sqlx::query_as::<_, (i32, f64)>(
            "SELECT attempts, extract(epoch FROM next_attempt_at - now())::float8 \
             FROM chat_messages",
        )
        .fetch_one(&c.app.pool)
        .await
        .unwrap()
    };
    run_job(&c).await;
    let (attempts, wait) = due_in().await;
    assert_eq!(attempts, 1);
    assert!((100.0..=121.0).contains(&wait), "{wait}");

    // Inside the backoff the job leaves the row alone.
    let calls = c.app.tone.calls();
    held::run(&c.app.state, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(c.app.tone.calls(), calls);

    for _ in 0..5 {
        run_job(&c).await;
    }
    let (attempts, wait) = due_in().await;
    assert_eq!(attempts, 6);
    assert!(
        (800.0..=901.0).contains(&wait),
        "capped at 15 minutes: {wait}"
    );
}

#[sqlx::test]
async fn a_poisoned_row_cannot_fill_the_batch(pool: PgPool) {
    let c = chat(pool).await;
    sqlx::query(
        "INSERT INTO chat_messages (conversation_id, sender_id, client_message_id, status, \
                body_enc, next_attempt_at) \
         VALUES ($1, $2, gen_random_uuid(), 'held', $3, now() - interval '1 hour')",
    )
    .bind(c.id)
    .bind(c.buyer_id)
    .bind(vec![1u8, 2, 3])
    .execute(&c.app.pool)
    .await
    .unwrap();
    // The row cannot be decrypted, so it fails, and is pushed back.
    let stats = held::run(&c.app.state, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(stats.failed, 1);
    let later: bool = sqlx::query_scalar("SELECT next_attempt_at > now() FROM chat_messages")
        .fetch_one(&c.app.pool)
        .await
        .unwrap();
    assert!(later);
}
