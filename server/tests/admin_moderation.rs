//! Owner and support moderation routes.

mod common;

use axum::http::Method;
use common::{Listing, TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    moderation::tone::ToneVerdict,
    payments::PaymentStatus,
};
use sqlx::PgPool;
use uuid::Uuid;

const SELLER: &str = "08055555551";
const BUYER: &str = "08022222221";
const METHOD: &str = "mono_directpay";
const PHONE_TEXT: &str = "call me on 08031234567";
const SLUG: &str = "kano-fabrics";

struct Desk {
    app: TestApp,
    shop: Listing,
    owner: String,
    support: String,
    buyer_id: Uuid,
    buyer: String,
    chat: Uuid,
}

async fn desk(pool: PgPool) -> Desk {
    let app = TestApp::new(pool).await;
    let shop = app
        .make_active_store_with_product(SELLER, "Kano Fabrics", 10_000, 5)
        .await;
    let owner = app.owner_token().await;
    let support = app.make_support().await;
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
    let chat = res.json["id"].as_str().unwrap().parse().unwrap();
    Desk {
        app,
        shop,
        owner,
        support,
        buyer_id,
        buyer,
        chat,
    }
}

impl Desk {
    async fn say(&self, body: &str) -> TestResponse {
        self.app
            .request(
                Method::POST,
                &format!("/chats/{}/messages", self.chat),
                Some(json!({ "client_message_id": Uuid::new_v4(), "body": body })),
                Some(&self.buyer),
            )
            .await
    }

    /// Blocks one message, and moves it out of the strike cooldown.
    async fn strike(&self) -> String {
        let res = self.say(PHONE_TEXT).await;
        assert_eq!(res.json["outcome"], "blocked", "{}", res.json);
        self.exec("UPDATE moderation_strikes SET at = at - interval '1 day'")
            .await;
        res.json["flag_id"].as_str().unwrap().to_string()
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(sql).execute(&self.app.pool).await.unwrap();
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        token: &str,
    ) -> TestResponse {
        self.app.request(method, path, body, Some(token)).await
    }

    async fn get(&self, path: &str, token: &str) -> TestResponse {
        self.call(Method::GET, path, None, token).await
    }

    async fn resolve(&self, flag: &str, action: &str, token: &str) -> TestResponse {
        self.call(
            Method::POST,
            &format!("/admin/moderation/flags/{flag}/resolve"),
            Some(json!({ "action": action })),
            token,
        )
        .await
    }

    async fn ban(&self, reason: &str) -> TestResponse {
        self.call(
            Method::POST,
            &format!("/admin/moderation/users/{}/ban", self.buyer_id),
            Some(json!({ "reason": reason })),
            &self.owner,
        )
        .await
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }

    async fn audits(&self, action: &str) -> Vec<Value> {
        sqlx::query_scalar("SELECT detail FROM audit_log WHERE action = $1")
            .bind(action)
            .fetch_all(&self.app.pool)
            .await
            .unwrap()
    }

    /// A delivered, released order, reviewed by the buyer.
    async fn reviewed(&self, stars: i64) -> (Uuid, String) {
        let app = &self.app;
        assert_eq!(
            app.set_cart(&self.buyer, self.shop.variant_id, 1)
                .await
                .status,
            200
        );
        let res = app.checkout(&self.buyer, METHOD).await;
        assert_eq!(res.status, 201, "{}", res.json);
        let reference = res.json["reference"].as_str().unwrap().to_string();
        let checkout: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
        app.fake.set_payment(
            &reference,
            PaymentStatus::Successful {
                amount_kobo: res.json["total_kobo"].as_i64().unwrap(),
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
        let order: Uuid = sqlx::query_scalar(
            "UPDATE orders SET status = 'completed', delivered_at = now() \
             WHERE checkout_id = $1 RETURNING id",
        )
        .bind(checkout)
        .fetch_one(&app.pool)
        .await
        .unwrap();
        let res = self
            .call(
                Method::POST,
                &format!("/orders/{order}/review"),
                Some(json!({ "stars": stars, "body": "lovely" })),
                &self.buyer,
            )
            .await;
        assert_eq!(res.json["outcome"], "posted", "{}", res.json);
        (
            order,
            res.json["review"]["id"].as_str().unwrap().to_string(),
        )
    }

    async fn store(&self) -> Value {
        self.app
            .request(Method::GET, &format!("/stores/{SLUG}"), None, None)
            .await
            .json
    }

    async fn public_reviews(&self) -> Value {
        self.app
            .request(Method::GET, &format!("/stores/{SLUG}/reviews"), None, None)
            .await
            .json
    }
}

#[sqlx::test]
async fn support_sees_metadata_but_not_originals(pool: PgPool) {
    let d = desk(pool).await;
    let flag = d.strike().await;
    let list = d.get("/admin/moderation/flags", &d.support).await;
    assert_eq!(list.status, 200, "{}", list.json);
    let item = &list.json["items"][0];
    assert_eq!(item["id"], flag.as_str());
    assert_eq!(item["surface"], "chat");
    assert_eq!(item["severity"], "serious");
    assert_eq!(item["category"], "off_platform");
    assert_eq!(item["source"], "filter");
    assert!(!list.json.to_string().contains("0803"));
    assert!(item.get("original").is_none());
    assert_eq!(d.audits("admin.viewed").await.len(), 1);

    let detail = format!("/admin/moderation/flags/{flag}");
    assert_eq!(d.get(&detail, &d.support).await.status, 403);
    assert_eq!(d.get(&detail, &d.buyer).await.status, 403);
    let plain = d.get("/admin/moderation/flags", &d.buyer).await;
    assert_eq!(plain.status, 403);
}

#[sqlx::test]
async fn flags_filter_and_page(pool: PgPool) {
    let d = desk(pool).await;
    let flag = d.strike().await;
    d.strike().await;
    d.strike().await;
    d.resolve(&flag, "uphold", &d.owner).await;

    let open = d
        .get("/admin/moderation/flags?status=open", &d.support)
        .await;
    assert_eq!(open.json["items"].as_array().unwrap().len(), 2);
    let mild = d
        .get("/admin/moderation/flags?severity=mild", &d.support)
        .await;
    assert_eq!(mild.json["items"], json!([]));
    let bad = d
        .get("/admin/moderation/flags?status=nope", &d.support)
        .await;
    assert_eq!(bad.status, 422);
    let bad = d
        .get("/admin/moderation/flags?severity=nope", &d.support)
        .await;
    assert_eq!(bad.status, 422);

    let page = d.get("/admin/moderation/flags?limit=2", &d.support).await;
    assert_eq!(page.json["items"].as_array().unwrap().len(), 2);
    let next = page.json["next_cursor"].as_str().unwrap();
    let rest = d
        .get(
            &format!("/admin/moderation/flags?limit=2&before={next}"),
            &d.support,
        )
        .await;
    assert_eq!(rest.json["items"].as_array().unwrap().len(), 1);
    assert!(rest.json["next_cursor"].is_null());
}

#[sqlx::test]
async fn owner_reads_original_and_it_is_audited(pool: PgPool) {
    let d = desk(pool).await;
    let flag = d.strike().await;
    let res = d
        .get(&format!("/admin/moderation/flags/{flag}"), &d.owner)
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["original"], PHONE_TEXT);
    let audits = d.audits("moderation.original_viewed").await;
    assert_eq!(audits.len(), 1);
    assert!(!audits[0].to_string().contains("0803"));
}

#[sqlx::test]
async fn owner_sees_the_suggestion_too(pool: PgPool) {
    let d = desk(pool).await;
    d.app
        .tone
        .set_next(Ok(ToneVerdict::mild("Please check this.")));
    let res = d.say("sort this out now").await;
    let flag = res.json["flag_id"].as_str().unwrap();
    let res = d
        .get(&format!("/admin/moderation/flags/{flag}"), &d.owner)
        .await;
    assert_eq!(res.json["original"], "sort this out now");
    assert_eq!(res.json["suggestion"], "Please check this.");
}

#[sqlx::test]
async fn unknown_flag_is_404(pool: PgPool) {
    let d = desk(pool).await;
    let flag = Uuid::new_v4().to_string();
    let path = format!("/admin/moderation/flags/{flag}");
    assert_eq!(d.get(&path, &d.owner).await.status, 404);
    assert_eq!(d.resolve(&flag, "dismiss", &d.owner).await.status, 404);
}

#[sqlx::test]
async fn dismiss_voids_the_strike_and_lifts_the_mute(pool: PgPool) {
    let d = desk(pool).await;
    d.strike().await;
    d.strike().await;
    let third = d.strike().await;
    assert_eq!(d.say("hello").await.status, 403, "muted after three");

    let res = d.resolve(&third, "dismiss", &d.support).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(
        d.scalar("SELECT count(*) FROM moderation_strikes WHERE voided_at IS NOT NULL")
            .await,
        1
    );
    assert_eq!(
        d.scalar("SELECT count(*) FROM user_moderation WHERE muted_until IS NOT NULL")
            .await,
        0
    );
    assert_eq!(d.say("hello").await.json["outcome"], "delivered");
    assert_eq!(d.audits("moderation.flag_resolved").await.len(), 1);
}

#[sqlx::test]
async fn dismiss_leaves_a_mute_earned_by_the_other_strikes(pool: PgPool) {
    let d = desk(pool).await;
    let first = d.strike().await;
    d.strike().await;
    d.strike().await;
    d.exec(
        "INSERT INTO moderation_strikes (user_id, category, at) \
            SELECT user_id, category, at FROM moderation_strikes LIMIT 1",
    )
    .await;
    assert_eq!(d.resolve(&first, "dismiss", &d.owner).await.status, 200);
    assert_eq!(d.say("hello").await.status, 403, "three strikes remain");
}

#[sqlx::test]
async fn dismiss_clears_owner_review_below_the_threshold(pool: PgPool) {
    let d = desk(pool).await;
    let flag = d.strike().await;
    sqlx::query("INSERT INTO user_moderation (user_id, owner_review_at) VALUES ($1, now())")
        .bind(d.buyer_id)
        .execute(&d.app.pool)
        .await
        .unwrap();
    let queue = d.get("/admin/moderation/review-queue", &d.support).await;
    assert_eq!(queue.status, 200, "{}", queue.json);
    assert_eq!(queue.json["items"][0]["user_id"], d.buyer_id.to_string());

    d.resolve(&flag, "dismiss", &d.owner).await;
    let queue = d.get("/admin/moderation/review-queue", &d.support).await;
    assert_eq!(queue.json["items"], json!([]));
}

#[sqlx::test]
async fn banned_user_leaves_the_review_queue(pool: PgPool) {
    let d = desk(pool).await;
    sqlx::query("INSERT INTO user_moderation (user_id, owner_review_at) VALUES ($1, now())")
        .bind(d.buyer_id)
        .execute(&d.app.pool)
        .await
        .unwrap();
    assert_eq!(d.ban("repeated scams").await.status, 200);
    let queue = d.get("/admin/moderation/review-queue", &d.support).await;
    assert_eq!(queue.json["items"], json!([]));
    assert_eq!(
        d.get("/admin/moderation/review-queue", &d.buyer)
            .await
            .status,
        403
    );
}

#[sqlx::test]
async fn dismiss_twice_is_409(pool: PgPool) {
    let d = desk(pool).await;
    let flag = d.strike().await;
    assert_eq!(d.resolve(&flag, "dismiss", &d.owner).await.status, 200);
    let again = d.resolve(&flag, "dismiss", &d.owner).await;
    assert_eq!(again.status, 409);
    assert_eq!(again.json["error"]["code"], "flag_resolved");
    assert_eq!(d.resolve(&flag, "uphold", &d.owner).await.status, 409);
}

#[sqlx::test]
async fn uphold_keeps_the_strike(pool: PgPool) {
    let d = desk(pool).await;
    let flag = d.strike().await;
    assert_eq!(d.resolve(&flag, "uphold", &d.support).await.status, 200);
    assert_eq!(
        d.scalar("SELECT count(*) FROM moderation_strikes WHERE voided_at IS NULL")
            .await,
        1
    );
    assert_eq!(d.resolve(&flag, "other", &d.owner).await.status, 422);
}

#[sqlx::test]
async fn dismissing_a_mild_flag_changes_nothing_else(pool: PgPool) {
    let d = desk(pool).await;
    d.strike().await;
    d.strike().await;
    d.strike().await;
    d.app.tone.set_next(Ok(ToneVerdict::mild("Please check.")));
    // The mute refuses before the check, so make the mild flag by hand first.
    d.exec("UPDATE user_moderation SET muted_until = NULL")
        .await;
    let res = d.say("sort this out now").await;
    let flag = res.json["flag_id"].as_str().unwrap().to_string();
    d.exec("UPDATE user_moderation SET muted_until = now() + interval '1 hour'")
        .await;

    assert_eq!(d.resolve(&flag, "dismiss", &d.owner).await.status, 200);
    assert_eq!(
        d.scalar("SELECT count(*) FROM moderation_strikes WHERE voided_at IS NULL")
            .await,
        3
    );
    assert_eq!(d.say("hello").await.status, 403, "still muted");
}

#[sqlx::test]
async fn unmute_is_owner_only(pool: PgPool) {
    let d = desk(pool).await;
    d.strike().await;
    d.strike().await;
    d.strike().await;
    let path = format!("/admin/moderation/users/{}/unmute", d.buyer_id);
    let res = d.call(Method::POST, &path, None, &d.support).await;
    assert_eq!(res.status, 403);
    let res = d.call(Method::POST, &path, None, &d.owner).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(d.say("hello").await.json["outcome"], "delivered");
    let again = d.call(Method::POST, &path, None, &d.owner).await;
    assert_eq!(again.status, 409);
    assert_eq!(again.json["error"]["code"], "not_muted");
    assert_eq!(d.audits("moderation.unmuted").await.len(), 1);
    let nobody = format!("/admin/moderation/users/{}/unmute", Uuid::new_v4());
    assert_eq!(
        d.call(Method::POST, &nobody, None, &d.owner).await.status,
        404
    );
}

#[sqlx::test]
async fn ban_blocks_posting_until_unban(pool: PgPool) {
    let d = desk(pool).await;
    let (order, _) = d.reviewed(5).await;
    let res = d.ban("repeated scams").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(d.ban("again").await.json["error"]["code"], "already_banned");

    let refused = d.say("hello").await;
    assert_eq!(refused.status, 403);
    assert_eq!(refused.json["error"]["code"], "posting_banned");
    let audits = d.audits("moderation.banned").await;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0]["reason"], "repeated scams");

    // Posting only: their order is still theirs.
    let path = format!("/orders/{order}");
    assert_eq!(d.get(&path, &d.buyer).await.status, 200);
    let tracking = d.get(&format!("{path}/tracking"), &d.buyer).await;
    assert_ne!(tracking.status, 403, "{}", tracking.json);

    let unban = format!("/admin/moderation/users/{}/unban", d.buyer_id);
    let res = d.call(Method::POST, &unban, None, &d.support).await;
    assert_eq!(res.status, 403);
    let res = d.call(Method::POST, &unban, None, &d.owner).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(d.say("hello").await.json["outcome"], "delivered");
    let again = d.call(Method::POST, &unban, None, &d.owner).await;
    assert_eq!(again.json["error"]["code"], "not_banned");
}

#[sqlx::test]
async fn ban_is_owner_only(pool: PgPool) {
    let d = desk(pool).await;
    let path = format!("/admin/moderation/users/{}/ban", d.buyer_id);
    let body = json!({ "reason": "no" });
    let res = d.call(Method::POST, &path, Some(body), &d.support).await;
    assert_eq!(res.status, 403);
}

#[sqlx::test]
async fn reason_with_an_account_number_is_refused(pool: PgPool) {
    let d = desk(pool).await;
    assert_eq!(d.ban("paid to 0123456785").await.status, 422);
    assert_eq!(d.say("hello").await.json["outcome"], "delivered");
}

#[sqlx::test]
async fn hide_review_updates_the_rating_and_public_list(pool: PgPool) {
    let d = desk(pool).await;
    d.reviewed(5).await;
    let (_, second) = d.reviewed(2).await;
    assert_eq!(d.store().await["rating_average"], 3.5);

    let path = format!("/admin/reviews/{second}/hide");
    assert_eq!(
        d.call(Method::POST, &path, None, &d.buyer).await.status,
        403
    );
    let res = d.call(Method::POST, &path, None, &d.support).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let store = d.store().await;
    assert_eq!(store["rating_count"], 1);
    assert_eq!(store["rating_average"], 5.0);
    assert_eq!(
        d.public_reviews().await["reviews"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let again = d.call(Method::POST, &path, None, &d.support).await;
    assert_eq!(again.json["error"]["code"], "review_hidden");
    assert_eq!(d.audits("review.hidden").await.len(), 1);
}

#[sqlx::test]
async fn unhide_restores_it(pool: PgPool) {
    let d = desk(pool).await;
    let (_, review) = d.reviewed(4).await;
    let unhide = format!("/admin/reviews/{review}/unhide");
    let res = d.call(Method::POST, &unhide, None, &d.support).await;
    assert_eq!(res.json["error"]["code"], "review_not_hidden");

    d.call(
        Method::POST,
        &format!("/admin/reviews/{review}/hide"),
        None,
        &d.support,
    )
    .await;
    assert_eq!(d.store().await["rating_count"], 0);
    assert!(d.store().await["rating_average"].is_null());
    let res = d.call(Method::POST, &unhide, None, &d.support).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(d.store().await["rating_count"], 1);
    assert_eq!(d.public_reviews().await["reviews"][0]["stars"], 4);
    let hidden_by = d
        .scalar("SELECT count(*) FROM reviews WHERE hidden_by IS NOT NULL")
        .await;
    assert_eq!(hidden_by, 0);
}

#[sqlx::test]
async fn hiding_an_unknown_review_is_404(pool: PgPool) {
    let d = desk(pool).await;
    let path = format!("/admin/reviews/{}/hide", Uuid::new_v4());
    assert_eq!(
        d.call(Method::POST, &path, None, &d.support).await.status,
        404
    );
}
