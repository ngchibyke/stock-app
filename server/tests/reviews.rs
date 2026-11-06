//! Reviews: one per completed delivery, moderated like chat, one seller reply.

mod common;

use axum::http::Method;
use common::{Listing, TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    escrow::jobs::run_due_jobs,
    limits::MAX_REVIEW_PHOTOS,
    moderation::tone::{ToneError, ToneVerdict},
    orders::service::mark_picked_up,
    payments::PaymentStatus,
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

const SELLER: &str = "08055555551";
const BUYER: &str = "08022222221";
const STRANGER: &str = "08033333331";
const METHOD: &str = "mono_directpay";
const PHONE_TEXT: &str = "call me on 08031234567";
const STORE: &str = "Kano Fabrics";
const SLUG: &str = "kano-fabrics";

struct Sale {
    app: TestApp,
    shop: Listing,
    buyer_id: Uuid,
    buyer: String,
}

async fn sale(pool: PgPool) -> Sale {
    let app = TestApp::new(pool).await;
    let shop = app
        .make_active_store_with_product(SELLER, STORE, 10_000, 5)
        .await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    Sale {
        app,
        shop,
        buyer_id,
        buyer,
    }
}

impl Sale {
    /// A paid order, through the real checkout and settlement.
    async fn paid_order(&self) -> Uuid {
        let app = &self.app;
        let res = app.set_cart(&self.buyer, self.shop.variant_id, 1).await;
        assert_eq!(res.status, 200, "{}", res.json);
        let res = app.checkout(&self.buyer, METHOD).await;
        assert_eq!(res.status, 201, "{}", res.json);
        let reference = res.json["reference"].as_str().unwrap().to_string();
        let total = res.json["total_kobo"].as_i64().unwrap();
        let checkout: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
        app.fake.set_payment(
            &reference,
            PaymentStatus::Successful {
                amount_kobo: total,
                fee_kobo: 10,
                currency: "NGN".into(),
                held: false,
                reference: reference.clone(),
            },
        );
        assert_eq!(
            settle_payment(&app.state, &reference).await.unwrap(),
            SettleOutcome::Paid
        );
        sqlx::query_scalar("SELECT id FROM orders WHERE checkout_id = $1")
            .bind(checkout)
            .fetch_one(&app.pool)
            .await
            .unwrap()
    }

    /// An order the money has been released on after a delivery.
    async fn completed_order(&self) -> Uuid {
        let order = self.paid_order().await;
        sqlx::query("UPDATE orders SET status = 'completed', delivered_at = now() WHERE id = $1")
            .bind(order)
            .execute(&self.app.pool)
            .await
            .unwrap();
        order
    }

    async fn post(&self, token: &str, order: Uuid, body: Value) -> TestResponse {
        self.app
            .request(
                Method::POST,
                &format!("/orders/{order}/review"),
                Some(body),
                Some(token),
            )
            .await
    }

    async fn review(&self, order: Uuid, body: Value) -> TestResponse {
        self.post(&self.buyer, order, body).await
    }

    async fn posted(&self, order: Uuid, body: Value) -> String {
        let res = self.review(order, body).await;
        assert_eq!(res.status, 200, "{}", res.json);
        assert_eq!(res.json["outcome"], "posted", "{}", res.json);
        res.json["review"]["id"].as_str().unwrap().to_string()
    }

    async fn reply(&self, token: &str, review: &str, body: Value) -> TestResponse {
        self.app
            .request(
                Method::POST,
                &format!("/reviews/{review}/reply"),
                Some(body),
                Some(token),
            )
            .await
    }

    async fn public_list(&self) -> TestResponse {
        self.app
            .request(Method::GET, &format!("/stores/{SLUG}/reviews"), None, None)
            .await
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }

    async fn reviews(&self) -> i64 {
        self.count("SELECT count(*) FROM reviews").await
    }

    async fn strikes(&self) -> i64 {
        self.count("SELECT count(*) FROM moderation_strikes").await
    }
}

#[sqlx::test]
async fn only_a_completed_delivery_can_be_reviewed(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.paid_order().await;
    for status in ["paid", "delivered", "refund_pending", "cancelled"] {
        sqlx::query(
            "UPDATE orders SET status = $1::order_status, delivered_at = now() WHERE id = $2",
        )
        .bind(status)
        .bind(order)
        .execute(&s.app.pool)
        .await
        .unwrap();
        let res = s.review(order, json!({ "stars": 5 })).await;
        assert_eq!(res.status, 409, "{status}: {}", res.json);
        assert_eq!(res.json["error"]["code"], "review_not_allowed");
    }
}

#[sqlx::test]
async fn release_by_owner_without_delivery_cannot_be_reviewed(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.paid_order().await;
    sqlx::query("UPDATE orders SET status = 'completed', delivered_at = NULL WHERE id = $1")
        .bind(order)
        .execute(&s.app.pool)
        .await
        .unwrap();
    let res = s.review(order, json!({ "stars": 5 })).await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "review_not_allowed");
}

#[sqlx::test]
async fn auto_released_order_can_be_reviewed(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.paid_order().await;
    let now = OffsetDateTime::now_utc();
    mark_picked_up(&s.app.pool, order, now).await.unwrap();
    s.app.mark_delivered(order, now).await.unwrap();
    sqlx::query("UPDATE orders SET release_at = now() - interval '1 minute' WHERE id = $1")
        .bind(order)
        .execute(&s.app.pool)
        .await
        .unwrap();
    assert_eq!(run_due_jobs(&s.app.state).await.released, 1);
    s.posted(order, json!({ "stars": 4 })).await;
}

#[sqlx::test]
async fn stranger_gets_404(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    let (_, stranger) = s.app.sign_up_verified(STRANGER).await;
    let res = s.post(&stranger, order, json!({ "stars": 5 })).await;
    assert_eq!(res.status, 404);
    let res = s
        .post(&s.shop.seller_token, order, json!({ "stars": 5 }))
        .await;
    assert_eq!(res.status, 404);
}

#[sqlx::test]
async fn one_review_per_order(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    let (a, b) = tokio::join!(
        s.review(order, json!({ "stars": 5 })),
        s.review(order, json!({ "stars": 1 }))
    );
    let mut statuses = [a.status, b.status];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 409]);
    let loser = if a.status == 409 { &a } else { &b };
    assert_eq!(loser.json["error"]["code"], "already_reviewed");
    assert_eq!(s.reviews().await, 1);
    let res = s.review(order, json!({ "stars": 3 })).await;
    assert_eq!(res.status, 409);
    let (count, sum): (i32, i64) =
        sqlx::query_as("SELECT rating_count, rating_sum FROM stores WHERE id = $1")
            .bind(s.shop.store_id)
            .fetch_one(&s.app.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert!(sum == 5 || sum == 1);
}

#[sqlx::test]
async fn stars_must_be_one_to_five(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    for stars in [0, 6, -1] {
        assert_eq!(s.review(order, json!({ "stars": stars })).await.status, 422);
    }
    assert_eq!(s.review(order, json!({})).await.status, 422);
}

#[sqlx::test]
async fn stars_only_review_skips_claude(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    s.posted(order, json!({ "stars": 5, "body": "   " })).await;
    assert_eq!(s.app.tone.calls(), 0);
    let list = s.public_list().await;
    assert!(list.json["reviews"][0]["body"].is_null());
}

#[sqlx::test]
async fn body_limit_counts_characters(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    let over = "é".repeat(2001);
    let res = s.review(order, json!({ "stars": 5, "body": over })).await;
    assert_eq!(res.status, 422);
    let full = "é".repeat(2000);
    s.posted(order, json!({ "stars": 5, "body": full })).await;
}

#[sqlx::test]
async fn photo_limit(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    let too_many: Vec<String> = (0..=MAX_REVIEW_PHOTOS)
        .map(|i| format!("p/{i}.jpg"))
        .collect();
    let res = s
        .review(order, json!({ "stars": 5, "photo_keys": too_many }))
        .await;
    assert_eq!(res.status, 422);
    let res = s
        .review(order, json!({ "stars": 5, "photo_keys": ["  "] }))
        .await;
    assert_eq!(res.status, 422);
    s.posted(order, json!({ "stars": 5, "photo_keys": ["p/a.jpg"] }))
        .await;
    let list = s.public_list().await;
    assert_eq!(list.json["reviews"][0]["photo_keys"], json!(["p/a.jpg"]));
}

#[sqlx::test]
async fn review_passes_the_same_checks(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    let res = s
        .review(order, json!({ "stars": 1, "body": PHONE_TEXT }))
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["outcome"], "blocked");
    assert_eq!(res.json["strikes"], 1);
    assert_eq!(s.strikes().await, 1);
    assert_eq!(s.reviews().await, 0);
    assert_eq!(s.app.tone.calls(), 0);

    s.app
        .tone
        .set_next(Ok(ToneVerdict::mild("It arrived late.")));
    let res = s
        .review(order, json!({ "stars": 1, "body": "useless late shop" }))
        .await;
    assert_eq!(res.json["outcome"], "rewrite");
    assert_eq!(res.json["suggested"], "It arrived late.");
    assert!(res.json["flag_id"].is_string());
    assert_eq!(s.reviews().await, 0, "nothing stored until accepted");
}

#[sqlx::test]
async fn suggestion_is_single_use_and_bound_to_this_order(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    let other = s.completed_order().await;
    s.app
        .tone
        .set_next(Ok(ToneVerdict::mild("It arrived late.")));
    let res = s
        .review(order, json!({ "stars": 2, "body": "useless late shop" }))
        .await;
    let flag = res.json["flag_id"].clone();

    let body = json!({ "stars": 2, "use_suggestion": flag });
    let wrong_order = s.review(other, body.clone()).await;
    assert_eq!(wrong_order.status, 404);

    let both = json!({ "stars": 2, "use_suggestion": flag, "body": "mine" });
    assert_eq!(s.review(order, both).await.status, 422);

    let res = s.review(order, body.clone()).await;
    assert_eq!(res.json["outcome"], "posted", "{}", res.json);
    assert_eq!(res.json["review"]["body"], "It arrived late.");

    // Once used, it cannot post on the other order, and the first is taken.
    assert_eq!(s.review(other, body.clone()).await.status, 404);
    let again = s.review(order, body).await;
    assert_eq!(again.status, 409);
    assert_eq!(again.json["error"]["code"], "already_reviewed");
}

#[sqlx::test]
async fn expired_suggestion_is_refused(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    s.app
        .tone
        .set_next(Ok(ToneVerdict::mild("It arrived late.")));
    let res = s
        .review(order, json!({ "stars": 2, "body": "useless late shop" }))
        .await;
    sqlx::query("UPDATE moderation_flags SET created_at = now() - interval '61 minutes'")
        .execute(&s.app.pool)
        .await
        .unwrap();
    let res = s
        .review(
            order,
            json!({ "stars": 2, "use_suggestion": res.json["flag_id"] }),
        )
        .await;
    assert_eq!(res.status, 409);
    assert_eq!(res.json["error"]["code"], "suggestion_expired");
}

#[sqlx::test]
async fn outage_returns_503_and_stores_nothing(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    s.app.tone.set_next(Err(ToneError::Unavailable));
    let res = s
        .review(order, json!({ "stars": 5, "body": "lovely" }))
        .await;
    assert_eq!(res.status, 503);
    assert_eq!(res.json["error"]["code"], "moderation_unavailable");
    assert_eq!(s.reviews().await, 0);
    assert_eq!(s.count("SELECT count(*) FROM moderation_flags").await, 0);
    s.posted(order, json!({ "stars": 5, "body": "lovely" }))
        .await;
}

#[sqlx::test]
async fn muted_buyer_cannot_review(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    sqlx::query(
        "INSERT INTO user_moderation (user_id, muted_until) \
         VALUES ($1, now() + interval '1 hour')",
    )
    .bind(s.buyer_id)
    .execute(&s.app.pool)
    .await
    .unwrap();
    let res = s
        .review(order, json!({ "stars": 5, "body": "lovely" }))
        .await;
    assert_eq!(res.status, 403);
    assert_eq!(res.json["error"]["code"], "muted");
    assert_eq!(s.app.tone.calls(), 0);
    assert_eq!(s.review(order, json!({ "stars": 5 })).await.status, 403);
}

#[sqlx::test]
async fn seller_replies_once(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    let review = s.posted(order, json!({ "stars": 5 })).await;
    let seller = s.shop.seller_token.clone();

    let res = s
        .reply(&s.buyer, &review, json!({ "body": "thanks" }))
        .await;
    assert_eq!(res.status, 404, "only the store's owner replies");

    let (a, b) = tokio::join!(
        s.reply(&seller, &review, json!({ "body": "thanks one" })),
        s.reply(&seller, &review, json!({ "body": "thanks two" }))
    );
    let mut statuses = [a.status, b.status];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 409]);
    let loser = if a.status == 409 { &a } else { &b };
    assert_eq!(loser.json["error"]["code"], "already_replied");

    let list = s.public_list().await;
    assert!(list.json["reviews"][0]["reply"].is_string());
    assert!(list.json["reviews"][0]["replied_at"].is_string());
}

#[sqlx::test]
async fn reply_passes_the_same_checks(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    let review = s.posted(order, json!({ "stars": 1 })).await;
    let seller = s.shop.seller_token.clone();

    let res = s
        .reply(&seller, &review, json!({ "body": PHONE_TEXT }))
        .await;
    assert_eq!(res.json["outcome"], "blocked");

    s.app
        .tone
        .set_next(Ok(ToneVerdict::mild("We are sorry about this.")));
    let res = s
        .reply(&seller, &review, json!({ "body": "you lie" }))
        .await;
    assert_eq!(res.json["outcome"], "rewrite");
    let res = s
        .reply(
            &seller,
            &review,
            json!({ "use_suggestion": res.json["flag_id"] }),
        )
        .await;
    assert_eq!(res.json["outcome"], "posted", "{}", res.json);
    assert_eq!(res.json["review"]["reply"], "We are sorry about this.");
}

#[sqlx::test]
async fn public_list_hides_buyer_identity(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    s.posted(order, json!({ "stars": 5, "body": "lovely" }))
        .await;
    let name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(s.buyer_id)
        .fetch_one(&s.app.pool)
        .await
        .unwrap();
    let res = s.public_list().await;
    assert_eq!(res.status, 200);
    let text = res.json.to_string();
    assert!(res.json["reviews"][0]["stars"] == 5, "{text}");
    assert!(!text.contains(&s.buyer_id.to_string()));
    assert!(!text.contains(BUYER));
    assert!(!text.contains(&name));
    assert!(!text.contains(&order.to_string()));
}

#[sqlx::test]
async fn reviews_page_newest_first(pool: PgPool) {
    let s = sale(pool).await;
    for stars in [1, 2, 3] {
        let order = s.completed_order().await;
        s.posted(order, json!({ "stars": stars })).await;
    }
    let res = s
        .app
        .request(
            Method::GET,
            &format!("/stores/{SLUG}/reviews?limit=2"),
            None,
            None,
        )
        .await;
    assert_eq!(res.json["reviews"].as_array().unwrap().len(), 2);
    assert_eq!(res.json["reviews"][0]["stars"], 3);
    let next = res.json["next_cursor"].as_str().unwrap().to_string();
    let res = s
        .app
        .request(
            Method::GET,
            &format!("/stores/{SLUG}/reviews?limit=2&before={next}"),
            None,
            None,
        )
        .await;
    assert_eq!(res.json["reviews"].as_array().unwrap().len(), 1);
    assert_eq!(res.json["reviews"][0]["stars"], 1);
    assert!(res.json["next_cursor"].is_null());
}

#[sqlx::test]
async fn rating_appears_on_the_store(pool: PgPool) {
    let s = sale(pool).await;
    let path = format!("/stores/{SLUG}");
    let store = || s.app.request(Method::GET, &path, None, None);
    let empty = store().await;
    assert_eq!(empty.json["rating_count"], 0);
    assert!(empty.json["rating_average"].is_null());

    let first = s.completed_order().await;
    s.posted(first, json!({ "stars": 5 })).await;
    let second = s.completed_order().await;
    s.posted(second, json!({ "stars": 2 })).await;
    let rated = store().await;
    assert_eq!(rated.json["rating_count"], 2);
    assert_eq!(rated.json["rating_average"], 3.5);
}

#[sqlx::test]
async fn hidden_store_reviews_404(pool: PgPool) {
    let s = sale(pool).await;
    sqlx::query("UPDATE stores SET status = 'suspended' WHERE id = $1")
        .bind(s.shop.store_id)
        .execute(&s.app.pool)
        .await
        .unwrap();
    assert_eq!(s.public_list().await.status, 404);
}

#[sqlx::test]
async fn hidden_reviews_are_left_out(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    s.posted(order, json!({ "stars": 5, "body": "lovely" }))
        .await;
    sqlx::query("UPDATE reviews SET hidden_at = now()")
        .execute(&s.app.pool)
        .await
        .unwrap();
    assert_eq!(s.public_list().await.json["reviews"], json!([]));
}

#[sqlx::test]
async fn review_is_audited_without_text(pool: PgPool) {
    let s = sale(pool).await;
    let order = s.completed_order().await;
    s.posted(order, json!({ "stars": 5, "body": "lovely" }))
        .await;
    let detail: Value = sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = 'review.posted' AND subject_id = $1",
    )
    .bind(order)
    .fetch_one(&s.app.pool)
    .await
    .unwrap();
    assert!(!detail.to_string().contains("lovely"));
}
