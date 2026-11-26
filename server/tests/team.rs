//! A store's team: invites by phone, accepting and declining them, and the owner
//! managing members (part 7e-1, task 4). The invite route must never tell whether
//! a phone has an account.

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use server::escrow::jobs::run_due_jobs;
use server::notify::{
    kinds::Kind,
    service::{self as notify, Audience},
};
use uuid::Uuid;

use sqlx::PgPool;

const OWNER_PHONE: &str = "08061000001";
const INVITEE_PHONE: &str = "08063000001";
const UNKNOWN_PHONE: &str = "08063000099";

/// An active store whose owner token is returned with it.
async fn shop(app: &TestApp) -> (Uuid, Uuid, String) {
    let l = app
        .make_active_store_with_product(OWNER_PHONE, "Team Shop", 10_000, 5)
        .await;
    (l.store_id, l.seller_id, l.seller_token)
}

async fn invite(app: &TestApp, token: &str, store: Uuid, body: Value) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/stores/{store}/invites"),
        Some(body),
        Some(token),
    )
    .await
}

async fn my_invites(app: &TestApp, token: &str) -> Vec<Value> {
    let res = app
        .request(Method::GET, "/me/invites", None, Some(token))
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    res.json.as_array().unwrap().clone()
}

async fn answer(app: &TestApp, token: &str, invite: &str, verb: &str) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/me/invites/{invite}/{verb}"),
        None,
        Some(token),
    )
    .await
}

fn id_of(res: &TestResponse) -> String {
    assert_eq!(res.status, 201, "{}", res.json);
    res.json["id"].as_str().unwrap().to_string()
}

async fn invite_notices(app: &TestApp) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT user_id FROM notifications WHERE kind = 'store_invite'")
        .fetch_all(&app.pool)
        .await
        .unwrap()
}

/// The owner invites `phone` as `role`; the invitee (who must have an account)
/// accepts. Returns the invitee's id and token.
async fn join(app: &TestApp, owner: &str, store: Uuid, phone: &str, role: &str) -> (Uuid, String) {
    let (id, token) = app.sign_up_verified(phone).await;
    let inv = id_of(&invite(app, owner, store, json!({ "phone": phone, "role": role })).await);
    let res = answer(app, &token, &inv, "accept").await;
    assert_eq!(res.status, 200, "{}", res.json);
    (id, token)
}

#[sqlx::test]
async fn invite_same_response_for_known_and_unknown_phone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;
    let (known, _) = app.sign_up_verified(INVITEE_PHONE).await;

    let a = invite(
        &app,
        &owner,
        store,
        json!({ "phone": INVITEE_PHONE, "role": "staff" }),
    )
    .await;
    let b = invite(
        &app,
        &owner,
        store,
        json!({ "phone": UNKNOWN_PHONE, "role": "staff" }),
    )
    .await;
    assert_eq!(a.status, 201, "{}", a.json);
    assert_eq!(a.status, b.status);
    let keys = |v: &Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(keys(&a.json), vec!["id".to_string()]);
    assert_eq!(keys(&a.json), keys(&b.json));
    assert_ne!(a.json["id"], b.json["id"]);
    assert_eq!(
        a.headers.get("cache-control"),
        b.headers.get("cache-control")
    );

    // Only the phone with an account is told, in the app, never by SMS.
    assert_eq!(invite_notices(&app).await, vec![known]);
    let sms_before = app.otp.sent_count();
    invite(
        &app,
        &owner,
        store,
        json!({ "phone": "08063000098", "role": "staff" }),
    )
    .await;
    assert_eq!(app.otp.sent_count(), sms_before);

    // The owner's list is the same shape for both.
    let team = app
        .request(
            Method::GET,
            &format!("/stores/{store}/team"),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(team.status, 200, "{}", team.json);
    let invites = team.json["invites"].as_array().unwrap();
    assert_eq!(invites.len(), 3);
    let lasts: Vec<&str> = invites
        .iter()
        .map(|i| i["phone_last4"].as_str().unwrap())
        .collect();
    assert!(
        lasts.contains(&"0001") && lasts.contains(&"0099"),
        "{lasts:?}"
    );
    for i in invites {
        assert_eq!(keys(i), keys(&invites[0]));
    }
}

#[sqlx::test]
async fn invite_visible_after_later_sign_up(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;
    let inv = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": INVITEE_PHONE, "role": "manager" }),
        )
        .await,
    );
    assert!(invite_notices(&app).await.is_empty());

    // Signs up a day later, with the number in another format.
    let (_, token) = app.sign_up_verified("+2348063000001").await;
    let list = my_invites(&app, &token).await;
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0]["id"], inv);
    assert_eq!(list[0]["role"], "manager");
    assert_eq!(list[0]["store"]["id"], store.to_string());
    assert_eq!(list[0]["store"]["name"], "Team Shop");
    assert!(list[0]["store"]["slug"].is_string());
    assert!(list[0]["branch"].is_null());
    assert!(list[0]["expires_at"].is_string());
    assert!(list[0].get("phone_last4").is_none(), "{list:?}");

    let res = answer(&app, &token, &inv, "accept").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(my_invites(&app, &token).await.is_empty());
}

#[sqlx::test]
async fn accept_by_other_user_is_404(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;
    let (_, other) = app.sign_up_verified("08063000050").await;
    let inv = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": INVITEE_PHONE, "role": "staff" }),
        )
        .await,
    );
    assert!(my_invites(&app, &other).await.is_empty());
    for verb in ["accept", "decline"] {
        let res = answer(&app, &other, &inv, verb).await;
        assert_eq!(res.status, 404, "{verb}: {}", res.json);
        // Nor can the owner who sent it.
        assert_eq!(answer(&app, &owner, &inv, verb).await.status, 404);
    }
    assert_eq!(
        answer(&app, &other, "not-a-uuid", "accept").await.status,
        404
    );

    // The phone's owner signs up later and is the only one who can accept it.
    let (_, invitee) = app.sign_up_verified(INVITEE_PHONE).await;
    assert_eq!(answer(&app, &invitee, &inv, "accept").await.status, 200);
    let members: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM store_members WHERE store_id = $1 AND removed_at IS NULL",
    )
    .bind(store)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(members, 1);
}

#[sqlx::test]
async fn expired_and_revoked_invites_are_404(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;
    let (_, token) = app.sign_up_verified(INVITEE_PHONE).await;

    let expired = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": INVITEE_PHONE, "role": "staff" }),
        )
        .await,
    );
    sqlx::query(
        "UPDATE store_invites SET created_at = now() - interval '8 days', \
             expires_at = now() - interval '1 day' WHERE id = $1::uuid",
    )
    .bind(&expired)
    .execute(&app.pool)
    .await
    .unwrap();
    assert!(my_invites(&app, &token).await.is_empty());
    assert_eq!(answer(&app, &token, &expired, "accept").await.status, 404);

    let revoked = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": INVITEE_PHONE, "role": "staff" }),
        )
        .await,
    );
    assert_eq!(my_invites(&app, &token).await.len(), 1);
    let res = app
        .request(
            Method::DELETE,
            &format!("/stores/{store}/invites/{revoked}"),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 204, "{}", res.json);
    assert!(my_invites(&app, &token).await.is_empty());
    assert_eq!(answer(&app, &token, &revoked, "accept").await.status, 404);
    // Revoking again is 404, and the invitee cannot revoke.
    let again = app
        .request(
            Method::DELETE,
            &format!("/stores/{store}/invites/{revoked}"),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(again.status, 404);
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_log WHERE action = 'store.invite_revoked' AND subject_id = $1",
    )
    .bind(store)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(audit, 1);

    // A declined invite is gone too.
    let declined = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": INVITEE_PHONE, "role": "staff" }),
        )
        .await,
    );
    assert_eq!(answer(&app, &token, &declined, "decline").await.status, 204);
    assert_eq!(answer(&app, &token, &declined, "accept").await.status, 404);
    assert!(my_invites(&app, &token).await.is_empty());
}

#[sqlx::test]
async fn accept_makes_member_not_seller(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;

    // A buyer accepts: a staff member, still not a seller.
    let (buyer, buyer_token) = join(&app, &owner, store, INVITEE_PHONE, "staff").await;
    let is_seller: bool = sqlx::query_scalar("SELECT is_seller FROM users WHERE id = $1")
        .bind(buyer)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(!is_seller);
    let mine = app
        .request(Method::GET, "/stores/mine", None, Some(&buyer_token))
        .await;
    assert_eq!(mine.status, 200, "{}", mine.json);
    assert_eq!(mine.json[0]["my_role"], "staff");
    let create = app
        .request(
            Method::POST,
            "/stores",
            Some(json!({
                "name": "Own Shop", "pickup_address": "1 Road", "district": "everything-else",
                "pickup_state": "FCT", "delivery_courier": false, "delivery_self": true,
            "pickup_area_council": "amac", "self_delivery_fee_kobo": 0,
            "business_phone": "08031234567"
            })),
            Some(&buyer_token),
        )
        .await;
    assert_eq!(create.status, 403, "{}", create.json);

    // A seller with one store of their own accepts, and may still open a second.
    let (seller, seller_token) = app.make_seller("08063000002").await;
    let store_body = |name: &str| {
        json!({
            "name": name, "pickup_address": "1 Road", "district": "everything-else",
            "pickup_state": "FCT", "delivery_courier": false, "delivery_self": true,
            "pickup_area_council": "amac", "self_delivery_fee_kobo": 0,
            "business_phone": "08031234567"
        })
    };
    let first = app
        .request(
            Method::POST,
            "/stores",
            Some(store_body("First")),
            Some(&seller_token),
        )
        .await;
    assert_eq!(first.status, 201, "{}", first.json);
    let inv = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": "08063000002", "role": "manager" }),
        )
        .await,
    );
    assert_eq!(
        answer(&app, &seller_token, &inv, "accept").await.status,
        200
    );
    let second = app
        .request(
            Method::POST,
            "/stores",
            Some(store_body("Second")),
            Some(&seller_token),
        )
        .await;
    assert_eq!(second.status, 201, "{}", second.json);
    let is_seller: bool = sqlx::query_scalar("SELECT is_seller FROM users WHERE id = $1")
        .bind(seller)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(is_seller);

    let added: Vec<Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = 'store.member_added' AND subject_id = $1 \
         ORDER BY id",
    )
    .bind(store)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(added.len(), 2);
    assert_eq!(added[0]["user_id"], buyer.to_string());
}

#[sqlx::test]
async fn already_member_409(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;
    let (member, _) = join(&app, &owner, store, INVITEE_PHONE, "staff").await;

    let res = invite(
        &app,
        &owner,
        store,
        json!({ "phone": INVITEE_PHONE, "role": "manager" }),
    )
    .await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "already_member");

    // The owner cannot invite themselves.
    let res = invite(
        &app,
        &owner,
        store,
        json!({ "phone": OWNER_PHONE, "role": "staff" }),
    )
    .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "cannot_invite_self");

    // A manager may not be tied to a branch.
    let branch: Uuid =
        sqlx::query_scalar("SELECT id FROM store_branches WHERE store_id = $1 AND position = 0")
            .bind(store)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let res = invite(
        &app,
        &owner,
        store,
        json!({ "phone": UNKNOWN_PHONE, "role": "manager", "branch_id": branch }),
    )
    .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "staff_only_branch");
    sqlx::query("UPDATE store_branches SET closed_at = now() WHERE id = $1")
        .bind(branch)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = invite(
        &app,
        &owner,
        store,
        json!({ "phone": UNKNOWN_PHONE, "role": "staff", "branch_id": branch }),
    )
    .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "branch_closed");

    // Removed, then invited again: a new live row, the old one kept.
    let res = app
        .request(
            Method::DELETE,
            &format!("/stores/{store}/members/{member}"),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(res.status, 204, "{}", res.json);
    let signed_in = app.sign_in(INVITEE_PHONE, common::TEST_PASSWORD).await;
    let token = signed_in.json["token"].as_str().unwrap().to_string();
    let inv = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": INVITEE_PHONE, "role": "manager" }),
        )
        .await,
    );
    assert_eq!(answer(&app, &token, &inv, "accept").await.status, 200);
    let rows: Vec<(String, bool)> = sqlx::query_as(
        "SELECT role::text, removed_at IS NULL FROM store_members WHERE user_id = $1 \
         ORDER BY created_at",
    )
    .bind(member)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![("staff".to_string(), false), ("manager".to_string(), true)]
    );
}

#[sqlx::test]
async fn team_cap_and_invite_rate_limit(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, owner_id, owner) = shop(&app).await;

    // 20 invites in a day, then 429.
    for n in 0..20 {
        let res = invite(
            &app,
            &owner,
            store,
            json!({ "phone": format!("0807{:07}", n), "role": "staff" }),
        )
        .await;
        assert_eq!(res.status, 201, "{n}: {}", res.json);
    }
    let res = invite(
        &app,
        &owner,
        store,
        json!({ "phone": "08070000099", "role": "staff" }),
    )
    .await;
    assert_eq!(res.status, 429, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "too_many_invites");
    // A day later the window has rolled on.
    sqlx::query(
        "UPDATE store_invites SET created_at = created_at - interval '25 hours', \
             expires_at = expires_at - interval '25 hours'",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let pending = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": INVITEE_PHONE, "role": "staff" }),
        )
        .await,
    );
    let (_, invitee) = app.sign_up_verified(INVITEE_PHONE).await;

    // A team of 50 is full: no new invites, and a pending one cannot be accepted.
    for n in 0..50u32 {
        let user: Uuid = sqlx::query_scalar(
            "INSERT INTO users (phone_enc, phone_idx, display_name, password_hash) \
             VALUES (decode('01', 'hex'), int4send($1), 'M', 'x') RETURNING id",
        )
        .bind(n as i32)
        .fetch_one(&app.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO store_members (store_id, user_id, role, added_by) \
             VALUES ($1, $2, 'staff', $3)",
        )
        .bind(store)
        .bind(user)
        .bind(owner_id)
        .execute(&app.pool)
        .await
        .unwrap();
    }
    let res = invite(
        &app,
        &owner,
        store,
        json!({ "phone": UNKNOWN_PHONE, "role": "staff" }),
    )
    .await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "team_full");
    let res = answer(&app, &invitee, &pending, "accept").await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "team_full");
}

#[sqlx::test]
async fn remove_member_ends_access(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let o = app.shipped_order("self", "picked_up").await;
    let (member, token) = join(&app, &o.seller_token, o.store_id, INVITEE_PHONE, "staff").await;
    let step = format!("/orders/{}/picked-up", o.order_id);
    let res = app.request(Method::POST, &step, None, Some(&token)).await;
    assert_eq!(res.status, 200, "{}", res.json);

    // Managers read the team; staff do not; only the owner manages it.
    let team = format!("/stores/{}/team", o.store_id);
    assert_eq!(
        app.request(Method::GET, &team, None, Some(&token))
            .await
            .status,
        403
    );
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{}/members/{member}", o.store_id),
            Some(json!({ "role": "manager" })),
            Some(&o.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["role"], "manager");
    let res = app.request(Method::GET, &team, None, Some(&token)).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["members"][0]["user_id"], member.to_string());
    assert_eq!(res.json["members"][0]["display_name"], "Ada");
    let res = app
        .request(
            Method::DELETE,
            &format!("/stores/{}/members/{member}", o.store_id),
            None,
            Some(&token),
        )
        .await;
    assert_eq!(res.status, 403, "a manager cannot manage the team");

    let res = app
        .request(
            Method::DELETE,
            &format!("/stores/{}/members/{member}", o.store_id),
            None,
            Some(&o.seller_token),
        )
        .await;
    assert_eq!(res.status, 204, "{}", res.json);
    // The very next request is refused, as for a stranger.
    let res = app.request(Method::POST, &step, None, Some(&token)).await;
    assert_eq!(res.status, 404, "{}", res.json);
    let (by, at): (Option<Uuid>, bool) = sqlx::query_as(
        "SELECT removed_by, removed_at IS NOT NULL FROM store_members WHERE user_id = $1",
    )
    .bind(member)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(by, Some(o.seller_id));
    assert!(at);
    for action in ["store.member_role_changed", "store.member_removed"] {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_log WHERE action = $1 AND subject_id = $2 \
               AND actor_id = $3",
        )
        .bind(action)
        .bind(o.store_id)
        .bind(o.seller_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
        assert_eq!(n, 1, "{action}");
    }
}

#[sqlx::test]
async fn deleted_account_ends_membership(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;
    let (member, _) = join(&app, &owner, store, INVITEE_PHONE, "staff").await;
    // A pending invite elsewhere for the same phone is left alone.
    let other = app
        .make_active_store_with_product("08061000002", "Other Shop", 10_000, 5)
        .await;
    let pending = id_of(
        &invite(
            &app,
            &other.seller_token,
            other.store_id,
            json!({ "phone": INVITEE_PHONE, "role": "staff" }),
        )
        .await,
    );

    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(member)
        .execute(&app.pool)
        .await
        .unwrap();

    let (removed_by, ended): (Option<Uuid>, bool) = sqlx::query_as(
        "SELECT removed_by, removed_at IS NOT NULL FROM store_members WHERE user_id = $1",
    )
    .bind(member)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(ended);
    assert_eq!(removed_by, Some(member));
    let audit: Vec<(Option<Uuid>, Value)> = sqlx::query_as(
        "SELECT actor_id, detail FROM audit_log WHERE action = 'store.member_removed' \
           AND subject_id = $1",
    )
    .bind(store)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].0, Some(member));
    assert_eq!(audit[0].1["user_id"], member.to_string());
    let live: bool =
        sqlx::query_scalar("SELECT revoked_at IS NULL FROM store_invites WHERE id = $1::uuid")
            .bind(&pending)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(live);
    let team = app
        .request(
            Method::GET,
            &format!("/stores/{store}/team"),
            None,
            Some(&owner),
        )
        .await;
    assert_eq!(team.json["members"].as_array().unwrap().len(), 0);
}

#[sqlx::test]
async fn audit_never_holds_full_phone(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;
    let (member, _) = join(&app, &owner, store, INVITEE_PHONE, "staff").await;
    let inv = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": UNKNOWN_PHONE, "role": "staff" }),
        )
        .await,
    );
    app.request(
        Method::DELETE,
        &format!("/stores/{store}/invites/{inv}"),
        None,
        Some(&owner),
    )
    .await;
    app.request(
        Method::DELETE,
        &format!("/stores/{store}/members/{member}"),
        None,
        Some(&owner),
    )
    .await;

    let sent: Vec<Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = 'store.invite_sent' ORDER BY id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["phone_last4"], "0099");
    assert_eq!(sent[1]["role"], "staff");
    let all: Vec<String> = sqlx::query_scalar(
        "SELECT action || ' ' || detail::text FROM audit_log WHERE action LIKE 'store.%'",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    for entry in &all {
        for needle in ["8063000001", "8063000099", "063000"] {
            assert!(!entry.contains(needle), "{entry}");
        }
    }
    // The invite row holds the blind index and last four, never the number.
    let row: String =
        sqlx::query_scalar("SELECT row_to_json(i)::text FROM store_invites i WHERE id = $1::uuid")
            .bind(&inv)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(
        !row.contains("8063000099") && !row.contains("063000"),
        "{row}"
    );
}

#[sqlx::test]
async fn old_invites_purged_after_30_days(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;
    let mut ids = Vec::new();
    for n in 0..6 {
        ids.push(id_of(
            &invite(
                &app,
                &owner,
                store,
                json!({ "phone": format!("0807{:07}", n), "role": "staff" }),
            )
            .await,
        ));
    }
    let old = "created_at = now() - interval '40 days', expires_at = now() - interval '33 days'";
    for (id, set) in [
        // Expired 33 days ago: purged.
        (&ids[0], old.to_string()),
        // Declined 31 days ago: purged.
        (&ids[1], format!("{old}, declined_at = now() - interval '31 days'")),
        // Revoked 31 days ago: purged.
        (&ids[2], format!("{old}, revoked_at = now() - interval '31 days'")),
        // Revoked 10 days ago: kept.
        (
            &ids[3],
            "created_at = now() - interval '12 days', expires_at = now() - interval '5 days',              revoked_at = now() - interval '10 days'"
                .to_string(),
        ),
        // Expired 29 days ago: kept.
        (
            &ids[4],
            "created_at = now() - interval '36 days', expires_at = now() - interval '29 days'"
                .to_string(),
        ),
        // Accepted long ago: kept (it explains a membership).
        (&ids[5], format!("{old}, accepted_at = now() - interval '39 days'")),
    ] {
        sqlx::query(&format!("UPDATE store_invites SET {set} WHERE id = $1::uuid"))
            .bind(id)
            .execute(&app.pool)
            .await
            .unwrap();
    }

    let report = run_due_jobs(&app.state).await;
    assert_eq!(report.invites_purged, 3);
    let mut left: Vec<String> =
        sqlx::query_scalar("SELECT id::text FROM store_invites ORDER BY id")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    left.sort();
    let mut expect = vec![ids[3].clone(), ids[4].clone(), ids[5].clone()];
    expect.sort();
    assert_eq!(left, expect);
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM job_runs WHERE job = 'purge_invites'")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(runs, 1);
}

#[sqlx::test]
async fn team_invariants_hold_in_sql(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (store, _, owner) = shop(&app).await;
    let inv = id_of(
        &invite(
            &app,
            &owner,
            store,
            json!({ "phone": UNKNOWN_PHONE, "role": "staff" }),
        )
        .await,
    );
    for bad in [
        "UPDATE store_invites SET declined_at = now(), revoked_at = now() WHERE id = $1::uuid",
        "UPDATE store_invites SET expires_at = created_at WHERE id = $1::uuid",
    ] {
        assert!(
            sqlx::query(bad)
                .bind(&inv)
                .execute(&app.pool)
                .await
                .is_err(),
            "{bad}"
        );
    }
    let (member, _) = join(&app, &owner, store, INVITEE_PHONE, "staff").await;
    for bad in [
        "UPDATE store_members SET removed_at = now() WHERE user_id = $1",
        "UPDATE store_members SET removed_by = user_id WHERE user_id = $1",
    ] {
        assert!(
            sqlx::query(bad)
                .bind(member)
                .execute(&app.pool)
                .await
                .is_err(),
            "{bad}"
        );
    }
}

// ---- Task 9: team notifications ----

/// Notices of `kind` raised for `order`, as the sorted users they went to.
async fn recipients(app: &TestApp, kind: &str, order: Uuid) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM notifications WHERE kind = $1 AND order_id = $2")
            .bind(kind)
            .bind(order)
            .fetch_all(&app.pool)
            .await
            .unwrap();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

async fn tied_to(app: &TestApp, store: Uuid, user: Uuid, branch: Uuid) {
    sqlx::query("UPDATE store_members SET branch_id = $1 WHERE store_id = $2 AND user_id = $3")
        .bind(branch)
        .bind(store)
        .bind(user)
        .execute(&app.pool)
        .await
        .unwrap();
}

async fn tell(app: &TestApp, kind: Kind, order: Uuid, audience: Audience) {
    let mut conn = app.pool.acquire().await.unwrap();
    notify::tell_team(&mut conn, kind, order, audience)
        .await
        .unwrap();
}

/// A paid order on the store's first branch, plus a second branch.
async fn order_with_team(app: &TestApp) -> (server_ids::Ids, Uuid) {
    let o = app.shipped_order("courier", "picked_up").await;
    let first: Uuid = sqlx::query_scalar("SELECT branch_id FROM orders WHERE id = $1")
        .bind(o.order_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let other: Uuid = sqlx::query_scalar(
        "INSERT INTO store_branches (store_id, name, pickup_address_enc, pickup_area_council, position) \
         SELECT store_id, 'Garki', pickup_address_enc, 'bwari', 1 FROM store_branches \
         WHERE id = $1 RETURNING id",
    )
    .bind(first)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    (
        server_ids::Ids {
            order: o.order_id,
            store: o.store_id,
            owner: o.seller_id,
            owner_token: o.seller_token,
            first,
        },
        other,
    )
}

mod server_ids {
    use uuid::Uuid;
    pub struct Ids {
        pub order: Uuid,
        pub store: Uuid,
        pub owner: Uuid,
        pub owner_token: String,
        pub first: Uuid,
    }
}

#[sqlx::test]
async fn new_order_reaches_owner_managers_and_branch_staff_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (t, other) = order_with_team(&app).await;
    let (manager, _) = join(&app, &t.owner_token, t.store, "08063000010", "manager").await;
    let (wide, _) = join(&app, &t.owner_token, t.store, "08063000011", "staff").await;
    let (here, _) = join(&app, &t.owner_token, t.store, "08063000012", "staff").await;
    let (away, _) = join(&app, &t.owner_token, t.store, "08063000013", "staff").await;
    tied_to(&app, t.store, here, t.first).await;
    tied_to(&app, t.store, away, other).await;
    // A stranger with their own store never hears of it.
    let l = app
        .make_active_store_with_product("08063000014", "Other Shop", 1_000, 1)
        .await;

    tell(&app, Kind::OrderNew, t.order, Audience::Order).await;

    let got = recipients(&app, "order_new", t.order).await;
    assert_eq!(got, sorted(vec![t.owner, manager, wide, here]));
    assert!(!got.contains(&away) && !got.contains(&l.seller_id));
}

#[sqlx::test]
async fn staff_of_a_closed_branch_get_no_order_notice(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (t, other) = order_with_team(&app).await;
    let (staff, _) = join(&app, &t.owner_token, t.store, "08063000020", "staff").await;
    tied_to(&app, t.store, staff, t.first).await;
    sqlx::query("UPDATE store_branches SET closed_at = now() WHERE id = $1")
        .bind(t.first)
        .execute(&app.pool)
        .await
        .unwrap();
    let _ = other;
    tell(&app, Kind::OrderNew, t.order, Audience::Order).await;
    assert_eq!(recipients(&app, "order_new", t.order).await, vec![t.owner]);
}

#[sqlx::test]
async fn dispute_reaches_owner_and_managers(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (t, _) = order_with_team(&app).await;
    let (manager, _) = join(&app, &t.owner_token, t.store, "08063000030", "manager").await;
    let (_staff, _) = join(&app, &t.owner_token, t.store, "08063000031", "staff").await;
    let dispute = Uuid::new_v4();
    let mut conn = app.pool.acquire().await.unwrap();
    // The notice row references the dispute, so make a real one.
    sqlx::query(
        "INSERT INTO disputes (id, order_id, buyer_id, store_id, reason, status, seller_respond_by)          SELECT $1, o.id, o.buyer_id, o.store_id, 'damaged', 'awaiting_seller', now() + interval '2 days'          FROM orders o WHERE o.id = $2",
    )
    .bind(dispute)
    .bind(t.order)
    .execute(&mut *conn)
    .await
    .unwrap();
    notify::tell_team_dispute(
        &mut conn,
        Kind::DisputeOpened,
        t.order,
        dispute,
        Audience::Managers,
    )
    .await
    .unwrap();
    let got: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM notifications WHERE kind = 'dispute_opened' AND dispute_id = $1",
    )
    .bind(dispute)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(sorted(got), sorted(vec![t.owner, manager]));
}

#[sqlx::test]
async fn payout_notice_owner_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (t, _) = order_with_team(&app).await;
    join(&app, &t.owner_token, t.store, "08063000040", "manager").await;
    join(&app, &t.owner_token, t.store, "08063000041", "staff").await;
    tell(&app, Kind::FundsReleased, t.order, Audience::OwnerOnly).await;
    assert_eq!(
        recipients(&app, "funds_released", t.order).await,
        vec![t.owner]
    );
}

#[sqlx::test]
async fn removed_member_gets_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (t, _) = order_with_team(&app).await;
    let (manager, _) = join(&app, &t.owner_token, t.store, "08063000050", "manager").await;
    let (staff, _) = join(&app, &t.owner_token, t.store, "08063000051", "staff").await;
    sqlx::query(
        "UPDATE store_members SET removed_at = now(), removed_by = $2 WHERE user_id = ANY($1)",
    )
    .bind(vec![manager, staff])
    .bind(t.owner)
    .execute(&app.pool)
    .await
    .unwrap();
    tell(&app, Kind::OrderNew, t.order, Audience::Order).await;
    tell(&app, Kind::FundsReleased, t.order, Audience::Managers).await;
    assert_eq!(recipients(&app, "order_new", t.order).await, vec![t.owner]);
    assert_eq!(
        recipients(&app, "funds_released", t.order).await,
        vec![t.owner]
    );
}
