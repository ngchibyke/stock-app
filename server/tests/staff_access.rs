//! `require` reads the owner and the live member rows on every call, and every
//! seller route answers each member by role (part 7e-1, tasks 2 and 3).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use server::{
    checkout::{SettleOutcome, settle_payment},
    error::AppError,
    orders::service,
    payments::PaymentStatus,
    stores::access::{Action, Role, covers_branch_sql, is_member_sql, require},
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

async fn user(pool: &PgPool, n: u8) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO users (phone_enc, phone_idx, display_name, password_hash) \
         VALUES (decode('01', 'hex'), decode($1, 'hex'), 'U', 'x') RETURNING id",
    )
    .bind(format!("{n:02x}"))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn store(pool: &PgPool, owner: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO stores (owner_id, name, slug, pickup_address_enc, pickup_state, \
            delivery_courier, delivery_self, status) \
         VALUES ($1, 'Shop', 'shop', decode('aabb', 'hex'), 'FCT', true, true, 'active') \
         RETURNING id",
    )
    .bind(owner)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn add_member(pool: &PgPool, store: Uuid, user: Uuid, role: &str, by: Uuid) {
    sqlx::query(
        "INSERT INTO store_members (store_id, user_id, role, added_by) \
         VALUES ($1, $2, $3::store_member_role, $4)",
    )
    .bind(store)
    .bind(user)
    .bind(role)
    .bind(by)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn require_404_for_stranger_and_removed_member(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let stranger = user(&pool, 2).await;
    let former = user(&pool, 3).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, former, "manager", owner).await;
    sqlx::query("UPDATE store_members SET removed_at = now(), removed_by = $1 WHERE user_id = $2")
        .bind(owner)
        .bind(former)
        .execute(&pool)
        .await
        .unwrap();

    let mut conn = pool.acquire().await.unwrap();
    for who in [stranger, former] {
        let err = require(&mut conn, store, who, Action::ViewStore)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound), "{err:?}");
    }
    // The owner is in, and a deleted store is gone even for the owner.
    let m = require(&mut conn, store, owner, Action::DeleteStore)
        .await
        .unwrap();
    assert_eq!(m.role, Role::Owner);
    sqlx::query("UPDATE stores SET deleted_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    let err = require(&mut conn, store, owner, Action::ViewStore)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound), "{err:?}");
}

#[sqlx::test]
async fn require_403_for_staff_on_manager_action(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let staff = user(&pool, 2).await;
    let manager = user(&pool, 3).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, staff, "staff", owner).await;
    add_member(&pool, store, manager, "manager", owner).await;

    let mut conn = pool.acquire().await.unwrap();
    let err = require(&mut conn, store, staff, Action::PublishProducts)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Forbidden("not_allowed")), "{err:?}");
    assert!(
        require(&mut conn, store, staff, Action::EditProducts)
            .await
            .is_ok()
    );
    assert!(
        require(&mut conn, store, manager, Action::PublishProducts)
            .await
            .is_ok()
    );
    let err = require(&mut conn, store, manager, Action::ManageTeam)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Forbidden("not_allowed")), "{err:?}");
}

#[sqlx::test]
async fn removal_takes_effect_on_next_call(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let staff = user(&pool, 2).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, staff, "staff", owner).await;

    let mut conn = pool.acquire().await.unwrap();
    require(&mut conn, store, staff, Action::ViewStore)
        .await
        .unwrap();
    sqlx::query("UPDATE store_members SET removed_at = now(), removed_by = $1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let err = require(&mut conn, store, staff, Action::ViewStore)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound), "{err:?}");
}

#[sqlx::test]
async fn is_member_sql_matches_owner_and_live_members_only(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let staff = user(&pool, 2).await;
    let stranger = user(&pool, 3).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, staff, "staff", owner).await;

    let sql = format!(
        "SELECT s.id FROM stores s WHERE s.id = $1 AND {}",
        is_member_sql("$2")
    );
    for (who, expected) in [(owner, true), (staff, true), (stranger, false)] {
        let found: Option<Uuid> = sqlx::query_scalar(&sql)
            .bind(store)
            .bind(who)
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert_eq!(found.is_some(), expected);
    }
}

#[sqlx::test]
async fn covers_branch_sql_follows_the_staff_tie(pool: PgPool) {
    let owner = user(&pool, 1).await;
    let manager = user(&pool, 2).await;
    let wide = user(&pool, 3).await;
    let tied = user(&pool, 4).await;
    let store = store(&pool, owner).await;
    add_member(&pool, store, manager, "manager", owner).await;
    add_member(&pool, store, wide, "staff", owner).await;
    let (mine, other): (Uuid, Uuid) = (Uuid::new_v4(), Uuid::new_v4());
    sqlx::query("INSERT INTO store_branches (id, store_id, name, pickup_address_enc, position) VALUES ($1, $2, 'A', decode('aa', 'hex'), 1)")
        .bind(mine)
        .bind(store)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO store_members (store_id, user_id, role, branch_id, added_by) \
         VALUES ($1, $2, 'staff', $3, $4)",
    )
    .bind(store)
    .bind(tied)
    .bind(mine)
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap();

    let sql = format!(
        "SELECT s.id FROM stores s WHERE s.id = $1 AND {}",
        covers_branch_sql("$2", "$3")
    );
    for (who, branch, expected) in [
        (owner, other, true),
        (manager, other, true),
        (wide, other, true),
        (tied, mine, true),
        (tied, other, false),
    ] {
        let found: Option<Uuid> = sqlx::query_scalar(&sql)
            .bind(store)
            .bind(who)
            .bind(branch)
            .fetch_optional(&pool)
            .await
            .unwrap();
        assert_eq!(found.is_some(), expected);
    }
}

// ---------------------------------------------------------------------------
// Seller routes by role (task 3).
// ---------------------------------------------------------------------------

const OWNER: usize = 0;
const MANAGER: usize = 1;
const WIDE: usize = 2;
const STAFF_A: usize = 3;
const STAFF_B: usize = 4;
const REMOVED: usize = 5;
const STRANGER: usize = 6;
const NAMES: [&str; 7] = [
    "owner", "manager", "wide", "staff_a", "staff_b", "removed", "stranger",
];
const SLUG: &str = "team-store";
const METHOD: &str = "mono_directpay";
const COMPLAINT: &str = "The zip was torn and the lining was stained when it arrived";

/// One store with two branches (A is the first, where orders go today), an owner,
/// a manager, store-wide staff, staff at A, staff at B (none of them sellers), a
/// removed manager and a stranger (both sellers with no store); a published
/// product and a buyer.
struct Team {
    app: TestApp,
    store_id: Uuid,
    branch_a: Uuid,
    branch_b: Uuid,
    ids: [Uuid; 7],
    tokens: [String; 7],
    published: Uuid,
    variant: Uuid,
    buyer: String,
}

async fn team(pool: PgPool) -> Team {
    let app = TestApp::new(pool).await;
    let shop = app
        .make_active_store_with_product("08061000001", "Team Store", 10_000, 50)
        .await;
    let branch_a: Uuid =
        sqlx::query_scalar("SELECT id FROM store_branches WHERE store_id = $1 AND position = 0")
            .bind(shop.store_id)
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
    let mut ids = vec![shop.seller_id];
    let mut tokens = vec![shop.seller_token.clone()];
    for n in 2..=7 {
        let phone = format!("0806100000{n}");
        // The removed member and the stranger are sellers, so the seller gate lets
        // them through to the store check (a non-seller gets `not_a_seller` first).
        let (id, token) = if n > 5 {
            app.make_seller(&phone).await
        } else {
            app.sign_up_verified(&phone).await
        };
        ids.push(id);
        tokens.push(token);
    }
    let owner = ids[OWNER];
    for (who, role, branch) in [
        (MANAGER, "manager", None),
        (WIDE, "staff", None),
        (STAFF_A, "staff", Some(branch_a)),
        (STAFF_B, "staff", Some(branch_b)),
        (REMOVED, "manager", None),
    ] {
        sqlx::query(
            "INSERT INTO store_members (store_id, user_id, role, branch_id, added_by) \
             VALUES ($1, $2, $3::store_member_role, $4, $5)",
        )
        .bind(shop.store_id)
        .bind(ids[who])
        .bind(role)
        .bind(branch)
        .bind(owner)
        .execute(&app.pool)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE store_members SET removed_at = now(), removed_by = $1 WHERE user_id = $2")
        .bind(owner)
        .bind(ids[REMOVED])
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, buyer) = app.sign_up_verified("08062000001").await;
    Team {
        store_id: shop.store_id,
        branch_a,
        branch_b,
        ids: ids.try_into().unwrap(),
        tokens: tokens.try_into().unwrap(),
        published: shop.product_id,
        variant: shop.variant_id,
        buyer,
        app,
    }
}

impl Team {
    async fn call(
        &self,
        who: usize,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> TestResponse {
        self.app
            .request(method, path, body, Some(&self.tokens[who]))
            .await
    }

    /// Each (actor, status) in turn; a 403 must be `not_allowed`.
    async fn check(&self, method: Method, path: &str, body: Option<Value>, cases: &[(usize, u16)]) {
        for &(who, status) in cases {
            let res = self.call(who, method.clone(), path, body.clone()).await;
            assert_eq!(
                res.status, status,
                "{method} {path} as {}: {}",
                NAMES[who], res.json
            );
            if status == 403 {
                assert_eq!(res.json["error"]["code"], "not_allowed", "{}", res.json);
            }
        }
    }

    /// A paid order from branch A, delivered by `method` (`self` or `courier`).
    async fn paid_order(&self, method: &str) -> Uuid {
        let app = &self.app;
        let res = app.set_cart(&self.buyer, self.variant, 1).await;
        assert_eq!(res.status, 200, "{}", res.json);
        let res = if method == "self" {
            app.checkout(&self.buyer, METHOD).await
        } else {
            let address = app.buyer_address(&self.buyer).await;
            let opts = app.delivery_options(&self.buyer, address).await;
            assert_eq!(opts.status, 200, "{}", opts.json);
            let option = common::group_option(&opts.json, "courier");
            app.checkout_with(&self.buyer, METHOD, address, std::slice::from_ref(&option))
                .await
        };
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
        let (order, branch): (Uuid, Uuid) =
            sqlx::query_as("SELECT id, branch_id FROM orders WHERE checkout_id = $1")
                .bind(checkout)
                .fetch_one(&app.pool)
                .await
                .unwrap();
        assert_eq!(branch, self.branch_a);
        order
    }

    /// A self-delivery order out for delivery (the service steps).
    async fn out_for_delivery(&self) -> Uuid {
        let order = self.paid_order("self").await;
        let now = OffsetDateTime::now_utc();
        service::mark_picked_up(&self.app.pool, order, now)
            .await
            .unwrap();
        let mut tx = self.app.pool.begin().await.unwrap();
        service::mark_out_for_delivery(&mut tx, &self.app.state.cipher, order, now)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        order
    }

    /// A buyer's open dispute on a delivered self-delivery order at branch A.
    async fn dispute(&self) -> Uuid {
        let order = self.out_for_delivery().await;
        let enc: Vec<u8> = sqlx::query_scalar("SELECT handover_code_enc FROM orders WHERE id = $1")
            .bind(order)
            .fetch_one(&self.app.pool)
            .await
            .unwrap();
        let code = self.app.state.cipher.decrypt(&enc).unwrap();
        service::deliver_with_code(
            &self.app.pool,
            &self.app.state.cipher,
            self.ids[OWNER],
            order,
            &code,
            OffsetDateTime::now_utc(),
        )
        .await
        .unwrap();
        let body = self
            .app
            .ready_photos_in(
                &self.buyer,
                "dispute",
                json!({ "reason": "damaged", "body": COMPLAINT, "photo_keys": ["evidence/1.jpg"] }),
            )
            .await;
        let res = self
            .app
            .request(
                Method::POST,
                &format!("/orders/{order}/dispute"),
                Some(body),
                Some(&self.buyer),
            )
            .await;
        assert_eq!(res.status, 201, "{}", res.json);
        res.json["id"].as_str().unwrap().parse().unwrap()
    }

    /// (order, review): the buyer's review of a completed order at branch A.
    async fn review(&self) -> (Uuid, Uuid) {
        let order = self.paid_order("self").await;
        sqlx::query("UPDATE orders SET status = 'completed', delivered_at = now() WHERE id = $1")
            .bind(order)
            .execute(&self.app.pool)
            .await
            .unwrap();
        let res = self
            .app
            .request(
                Method::POST,
                &format!("/orders/{order}/review"),
                Some(json!({ "stars": 5 })),
                Some(&self.buyer),
            )
            .await;
        assert_eq!(res.status, 200, "{}", res.json);
        (
            order,
            res.json["review"]["id"].as_str().unwrap().parse().unwrap(),
        )
    }

    /// Its stock is at the maker's own branch (B for staff at B, else A): with two
    /// open branches stock is per branch, and staff count only their own.
    async fn new_product_body(&self, who: usize) -> Value {
        let photo = self.app.ready_photo_for(&self.tokens[who], "product").await;
        let branch = if who == STAFF_B {
            self.branch_b
        } else {
            self.branch_a
        };
        json!({
            "name": "Adire Scarf",
            "description": "Hand dyed",
            "category": "fashion",
            "condition": "new",
            "photo_keys": [photo],
            "weight_grams": 200,
            "base_price_kobo": 8_000,
            "variants": [{ "attributes": {}, "stock": { branch.to_string(): 3 } }]
        })
    }

    /// An unpublished product made by the owner.
    async fn unpublished(&self) -> Uuid {
        let body = self.new_product_body(OWNER).await;
        let res = self
            .call(
                OWNER,
                Method::POST,
                &format!("/stores/{}/products", self.store_id),
                Some(body),
            )
            .await;
        assert_eq!(res.status, 201, "{}", res.json);
        assert_eq!(res.json["is_published"], false);
        res.json["id"].as_str().unwrap().parse().unwrap()
    }

    async fn audit_actors(&self, action: &str, subject: Uuid) -> Vec<Option<Uuid>> {
        sqlx::query_scalar(
            "SELECT actor_id FROM audit_log WHERE action = $1 AND subject_id = $2 ORDER BY id",
        )
        .bind(action)
        .bind(subject)
        .fetch_all(&self.app.pool)
        .await
        .unwrap()
    }
}

fn members_then(outsiders: &[(usize, u16)], members: &[usize], status: u16) -> Vec<(usize, u16)> {
    let mut cases: Vec<(usize, u16)> = members.iter().map(|&w| (w, status)).collect();
    cases.extend_from_slice(outsiders);
    cases
}

#[sqlx::test]
async fn seller_route_permission_matrix(pool: PgPool) {
    let t = team(pool).await;
    let store = t.store_id;
    let all_members = [OWNER, MANAGER, WIDE, STAFF_A, STAFF_B];
    let gone = [(REMOVED, 404), (STRANGER, 404)];

    // --- A pending store: every member may walk in; nobody else sees it.
    sqlx::query("UPDATE stores SET status = 'pending' WHERE id = $1")
        .bind(store)
        .execute(&t.app.pool)
        .await
        .unwrap();
    let hidden = t.unpublished().await;
    for path in [
        format!("/stores/{SLUG}"),
        format!("/stores/{SLUG}/room"),
        format!("/products/{hidden}"),
    ] {
        t.check(
            Method::GET,
            &path,
            None,
            &members_then(&gone, &all_members, 200),
        )
        .await;
    }
    for who in all_members {
        let res = t
            .call(who, Method::GET, &format!("/stores/{SLUG}/products"), None)
            .await;
        assert_eq!(res.json.as_array().unwrap().len(), 2, "{}", NAMES[who]);
        assert!(
            res.json[0]["variants"][0]["stock"].is_number(),
            "{}",
            res.json
        );
    }
    let res = t
        .call(
            STRANGER,
            Method::GET,
            &format!("/stores/{SLUG}/products"),
            None,
        )
        .await;
    assert_eq!(res.json.as_array().unwrap().len(), 0, "{}", res.json);
    sqlx::query("UPDATE stores SET status = 'active' WHERE id = $1")
        .bind(store)
        .execute(&t.app.pool)
        .await
        .unwrap();

    // --- Store details: managers edit name and pictures, never owner fields.
    let path = format!("/stores/{store}");
    t.check(
        Method::PATCH,
        &path,
        Some(json!({ "name": "Team Store" })),
        &[
            (WIDE, 403),
            (STAFF_A, 403),
            (STAFF_B, 403),
            (REMOVED, 404),
            (STRANGER, 404),
            (MANAGER, 200),
            (OWNER, 200),
        ],
    )
    .await;
    for body in [
        json!({ "district": "everything-else" }),
        json!({ "name": "Team Store", "self_delivery_fee_kobo": 0 }),
        json!({ "business_phone": "08031234567" }),
        json!({ "pickup_address": "12 Unique Street, Wuse 2" }),
        json!({ "delivery_self": true }),
    ] {
        t.check(
            Method::PATCH,
            &path,
            Some(body),
            &[(MANAGER, 403), (WIDE, 403), (STRANGER, 404), (OWNER, 200)],
        )
        .await;
    }
    t.check(
        Method::DELETE,
        &path,
        None,
        &[
            (MANAGER, 403),
            (WIDE, 403),
            (STAFF_A, 403),
            (REMOVED, 404),
            (STRANGER, 404),
        ],
    )
    .await;
    t.check(
        Method::GET,
        &format!("/stores/{store}/payout-account"),
        None,
        &[
            (MANAGER, 403),
            (WIDE, 403),
            (STAFF_B, 403),
            (REMOVED, 404),
            (STRANGER, 404),
            (OWNER, 200),
        ],
    )
    .await;

    // --- Products: anyone in the team adds one; it starts unpublished.
    for (who, status) in members_then(&gone, &all_members, 201) {
        let body = t.new_product_body(who).await;
        let res = t
            .call(
                who,
                Method::POST,
                &format!("/stores/{store}/products"),
                Some(body),
            )
            .await;
        assert_eq!(res.status, status, "{}: {}", NAMES[who], res.json);
        if status == 201 {
            assert_eq!(res.json["is_published"], false);
        }
    }

    // A published product: other fields are open to staff; price and publish are not.
    let live = format!("/products/{}", t.published);
    t.check(
        Method::PATCH,
        &live,
        Some(json!({ "name": "Ankara Shirt 2" })),
        &members_then(&gone, &all_members, 200),
    )
    .await;
    // Restating the current price and flag is no change.
    t.check(
        Method::PATCH,
        &live,
        Some(json!({ "base_price_kobo": 10_000, "is_published": true })),
        &[(WIDE, 200), (STAFF_A, 200)],
    )
    .await;
    t.check(
        Method::PATCH,
        &live,
        Some(json!({ "base_price_kobo": 12_000 })),
        &[
            (WIDE, 403),
            (STAFF_A, 403),
            (STAFF_B, 403),
            (REMOVED, 404),
            (STRANGER, 404),
            (MANAGER, 200),
            (OWNER, 200),
        ],
    )
    .await;
    t.check(
        Method::PATCH,
        &live,
        Some(json!({ "is_published": false })),
        &[(WIDE, 403), (STAFF_A, 403), (STAFF_B, 403), (STRANGER, 404)],
    )
    .await;
    let stock = t.app.stock(t.variant).await;
    let variants = format!("{live}/variants");
    // Two open branches: stock is given per branch. All of it is at A.
    let at_a = |n: i32| json!({ t.branch_a.to_string(): n });
    // Same variants at the same prices, new stock at A: staff who cover A may.
    t.check(
        Method::PUT,
        &variants,
        Some(json!([{ "attributes": {}, "stock": at_a(stock + 1) }])),
        // Staff at B first: once A holds the new count, restating it is no change.
        &[
            (STAFF_B, 403),
            (WIDE, 200),
            (STAFF_A, 200),
            (REMOVED, 404),
            (STRANGER, 404),
        ],
    )
    .await;
    let stock = stock + 1;
    for body in [
        json!([{ "attributes": {}, "stock": at_a(stock), "price_kobo": 15_000 }]),
        json!([
            { "attributes": { "size": "M" }, "stock": at_a(stock) },
            { "attributes": { "size": "L" }, "stock": at_a(1) }
        ]),
    ] {
        t.check(
            Method::PUT,
            &variants,
            Some(body),
            &[(WIDE, 403), (STAFF_A, 403), (STAFF_B, 403), (STRANGER, 404)],
        )
        .await;
    }
    t.check(
        Method::PUT,
        &variants,
        Some(json!([{ "attributes": {}, "stock": at_a(stock), "price_kobo": 15_000 }])),
        &[(MANAGER, 200)],
    )
    .await;

    // An unpublished product: staff set prices and variants, but cannot publish.
    let draft = t.unpublished().await;
    let draft_path = format!("/products/{draft}");
    t.check(
        Method::PATCH,
        &draft_path,
        Some(json!({ "base_price_kobo": 9_000 })),
        &[(STAFF_A, 200), (WIDE, 200), (STRANGER, 404)],
    )
    .await;
    // The draft's stock is at A, so replacing its variants changes A's stock:
    // staff at A may, staff at B may not (task 5 closed this gap; it was 200).
    t.check(
        Method::PUT,
        &format!("{draft_path}/variants"),
        Some(json!([
            { "attributes": { "size": "M" }, "stock": at_a(1), "price_kobo": 9_500 },
            { "attributes": { "size": "L" }, "stock": at_a(1) }
        ])),
        &[(STAFF_B, 403), (STAFF_A, 200)],
    )
    .await;
    t.check(
        Method::PATCH,
        &draft_path,
        Some(json!({ "is_published": true })),
        &[
            (WIDE, 403),
            (STAFF_A, 403),
            (REMOVED, 404),
            (STRANGER, 404),
            (MANAGER, 200),
        ],
    )
    .await;
    // Deleting is a manager's.
    let doomed = t.unpublished().await;
    t.check(
        Method::DELETE,
        &format!("/products/{doomed}"),
        None,
        &[
            (WIDE, 403),
            (STAFF_A, 403),
            (REMOVED, 404),
            (STRANGER, 404),
            (MANAGER, 204),
        ],
    )
    .await;

    // --- Orders: a paid order at branch A.
    let order = t.paid_order("self").await;
    let not_b = [OWNER, MANAGER, WIDE, STAFF_A];
    let outsiders = [(STAFF_B, 404), (REMOVED, 404), (STRANGER, 404)];
    for path in [
        format!("/orders/{order}"),
        format!("/orders/{order}/tracking"),
    ] {
        t.check(
            Method::GET,
            &path,
            None,
            &members_then(&outsiders, &not_b, 200),
        )
        .await;
    }
    let list = format!("/stores/{store}/orders");
    t.check(
        Method::GET,
        &list,
        None,
        &members_then(&gone, &all_members, 200),
    )
    .await;
    for (who, sees) in [
        (OWNER, 1),
        (MANAGER, 1),
        (WIDE, 1),
        (STAFF_A, 1),
        (STAFF_B, 0),
    ] {
        let res = t.call(who, Method::GET, &list, None).await;
        assert_eq!(
            res.json["items"].as_array().unwrap().len(),
            sees,
            "{}: {}",
            NAMES[who],
            res.json
        );
    }
    let mut cases = outsiders.to_vec();
    cases.extend(not_b.iter().map(|&w| (w, 200)));
    t.check(
        Method::POST,
        &format!("/orders/{order}/picked-up"),
        None,
        &cases,
    )
    .await;

    // --- A dispute on a delivered order at branch A.
    let dispute_id = t.dispute().await;
    let dispute = dispute_id.to_string();
    t.check(
        Method::GET,
        &format!("/disputes/{dispute}"),
        None,
        &members_then(&outsiders, &not_b, 200),
    )
    .await;
    // Answering is a manager's: an empty answer shows who got past the gate (422).
    let respond = format!("/disputes/{dispute}/response");
    t.check(
        Method::POST,
        &respond,
        Some(json!({ "body": "" })),
        &[
            (WIDE, 403),
            (STAFF_A, 403),
            (STAFF_B, 403),
            (REMOVED, 404),
            (STRANGER, 404),
            (OWNER, 422),
            (MANAGER, 422),
        ],
    )
    .await;
    t.check(
        Method::POST,
        &respond,
        Some(json!({ "body": "We packed it with care; please send it back." })),
        &[(MANAGER, 200)],
    )
    .await;
    assert_eq!(
        t.audit_actors("dispute.seller_responded", dispute_id).await,
        vec![Some(t.ids[MANAGER])]
    );
    // The parcel comes back to the branch: anyone who may act on the order.
    let received = format!("/disputes/{dispute}/return-received");
    let awaiting_return = || async {
        sqlx::query(
            "UPDATE disputes SET status = 'awaiting_return', return_paid_by = 'seller', \
             return_requested_at = now(), return_received_at = NULL WHERE id = $1",
        )
        .bind(dispute_id)
        .execute(&t.app.pool)
        .await
        .unwrap();
    };
    awaiting_return().await;
    t.check(Method::POST, &received, None, &outsiders).await;
    for who in [STAFF_A, WIDE, MANAGER, OWNER] {
        awaiting_return().await;
        t.check(Method::POST, &received, None, &[(who, 200)]).await;
    }
    let actors = t.audit_actors("dispute.return_received", dispute_id).await;
    assert_eq!(actors[0], Some(t.ids[STAFF_A]));

    // --- A review: replying is a manager's.
    let (reviewed, review) = t.review().await;
    let reply = format!("/reviews/{review}/reply");
    t.check(
        Method::POST,
        &reply,
        Some(json!({ "body": "Thank you" })),
        &[
            (WIDE, 403),
            (STAFF_A, 403),
            (STAFF_B, 403),
            (REMOVED, 404),
            (STRANGER, 404),
            (MANAGER, 200),
        ],
    )
    .await;
    sqlx::query("UPDATE reviews SET reply = NULL, replied_at = NULL WHERE id = $1")
        .bind(review)
        .execute(&t.app.pool)
        .await
        .unwrap();
    t.check(
        Method::POST,
        &reply,
        Some(json!({ "body": "Thank you" })),
        &[(OWNER, 200)],
    )
    .await;
    assert_eq!(
        t.audit_actors("review.replied", reviewed).await,
        vec![Some(t.ids[MANAGER]), Some(t.ids[OWNER])]
    );
}

#[sqlx::test]
async fn member_cannot_buy_from_own_store(pool: PgPool) {
    let t = team(pool).await;
    for who in [MANAGER, WIDE, STAFF_B] {
        let res = t.app.set_cart(&t.tokens[who], t.variant, 1).await;
        assert_eq!(res.status, 422, "{}: {}", NAMES[who], res.json);
        assert_eq!(res.json["error"]["code"], "own_store_item");
    }
    // A line that was in the cart before the member joined.
    sqlx::query("INSERT INTO cart_items (user_id, variant_id, quantity) VALUES ($1, $2, 1)")
        .bind(t.ids[STAFF_A])
        .bind(t.variant)
        .execute(&t.app.pool)
        .await
        .unwrap();
    let address = t.app.buyer_address(&t.tokens[STAFF_A]).await;
    let res = t.app.delivery_options(&t.tokens[STAFF_A], address).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["stores"], json!([]), "{}", res.json);
    let res = t
        .app
        .checkout_with(&t.tokens[STAFF_A], METHOD, address, &[])
        .await;
    assert_eq!(res.json["error"]["code"], "own_store_item", "{}", res.json);
    // A removed member is a customer again.
    let res = t.app.set_cart(&t.tokens[REMOVED], t.variant, 1).await;
    assert_eq!(res.status, 200, "{}", res.json);
}

#[sqlx::test]
async fn stores_mine_lists_member_stores_with_role(pool: PgPool) {
    let t = team(pool).await;
    for (who, role, branch) in [
        (OWNER, "owner", None),
        (MANAGER, "manager", None),
        (WIDE, "staff", None),
        (STAFF_A, "staff", Some(t.branch_a)),
        (STAFF_B, "staff", Some(t.branch_b)),
    ] {
        let res = t.call(who, Method::GET, "/stores/mine", None).await;
        assert_eq!(res.status, 200, "{}: {}", NAMES[who], res.json);
        let stores = res.json.as_array().unwrap();
        assert_eq!(stores.len(), 1, "{}: {}", NAMES[who], res.json);
        assert_eq!(stores[0]["id"], json!(t.store_id));
        assert_eq!(stores[0]["my_role"], role, "{}", NAMES[who]);
        assert_eq!(stores[0]["my_branch_id"], json!(branch), "{}", NAMES[who]);
    }
    // A removed member sees the store no more.
    let res = t.call(REMOVED, Method::GET, "/stores/mine", None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json, json!([]));
    // A plain buyer, in no team, is not a seller.
    let res = t
        .app
        .request(Method::GET, "/stores/mine", None, Some(&t.buyer))
        .await;
    assert_eq!(res.status, 403, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "not_a_seller");
}

#[sqlx::test]
async fn step_records_member_as_actor(pool: PgPool) {
    let t = team(pool).await;
    sqlx::query("UPDATE users SET display_name = 'Amina' WHERE id = $1")
        .bind(t.ids[STAFF_A])
        .execute(&t.app.pool)
        .await
        .unwrap();
    let order = t.paid_order("self").await;
    let res = t
        .call(
            STAFF_A,
            Method::POST,
            &format!("/orders/{order}/picked-up"),
            None,
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let amina = json!({ "user_id": t.ids[STAFF_A], "display_name": "Amina" });
    assert_eq!(res.json["done_by"]["picked_up"], amina, "{}", res.json);
    let res = t
        .call(
            MANAGER,
            Method::POST,
            &format!("/orders/{order}/out-for-delivery"),
            None,
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["done_by"]["picked_up"], amina);
    assert_eq!(
        res.json["done_by"]["out_for_delivery"]["user_id"],
        json!(t.ids[MANAGER])
    );
    assert_eq!(
        t.audit_actors("order.seller_picked_up", order).await,
        vec![Some(t.ids[STAFF_A])]
    );
    assert_eq!(
        t.audit_actors("order.seller_out_for_delivery", order).await,
        vec![Some(t.ids[MANAGER])]
    );
    // The seller list says the same; the buyer's view does not name the team.
    let res = t
        .call(
            OWNER,
            Method::GET,
            &format!("/stores/{}/orders", t.store_id),
            None,
        )
        .await;
    assert_eq!(res.json["items"][0]["done_by"]["picked_up"], amina);
    let res = t
        .app
        .request(
            Method::GET,
            &format!("/orders/{order}"),
            None,
            Some(&t.buyer),
        )
        .await;
    assert!(res.json.get("done_by").is_none(), "{}", res.json);

    // A courier order's ready step, by store-wide staff.
    let courier = t.paid_order("courier").await;
    let res = t
        .call(
            WIDE,
            Method::POST,
            &format!("/orders/{courier}/ready"),
            None,
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["done_by"]["ready"]["user_id"], json!(t.ids[WIDE]));
    assert_eq!(
        t.audit_actors("order.ready", courier).await,
        vec![Some(t.ids[WIDE])]
    );
}

#[sqlx::test]
async fn contact_reveal_audited_per_member(pool: PgPool) {
    let t = team(pool).await;
    let order = t.out_for_delivery().await;
    let path = format!("/orders/{order}/tracking");
    for who in [OWNER, STAFF_A, STAFF_A, MANAGER] {
        let res = t.call(who, Method::GET, &path, None).await;
        assert_eq!(res.status, 200, "{}: {}", NAMES[who], res.json);
        assert!(
            res.json["buyer_contact"]["phone"].is_string(),
            "{}",
            res.json
        );
    }
    // Staff at another branch never get it, nor does anyone outside the team.
    for who in [STAFF_B, REMOVED, STRANGER] {
        let res = t.call(who, Method::GET, &path, None).await;
        assert_eq!(res.status, 404, "{}: {}", NAMES[who], res.json);
    }
    assert_eq!(
        t.audit_actors("order.contact_revealed", order).await,
        vec![
            Some(t.ids[OWNER]),
            Some(t.ids[STAFF_A]),
            Some(t.ids[MANAGER])
        ]
    );
    let details: Vec<Value> = sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = 'order.contact_revealed' AND subject_id = $1",
    )
    .bind(order)
    .fetch_all(&t.app.pool)
    .await
    .unwrap();
    for d in details {
        assert_eq!(d, json!({ "kind": "buyer_contact" }));
    }
}

#[sqlx::test]
async fn member_never_sees_contact_on_courier_order(pool: PgPool) {
    let t = team(pool).await;
    let courier = t.paid_order("courier").await;
    // A self-delivery order that has not left yet shows nothing either.
    let waiting = t.paid_order("self").await;
    for order in [courier, waiting] {
        for who in [OWNER, MANAGER, WIDE, STAFF_A] {
            for path in [
                format!("/orders/{order}/tracking"),
                format!("/orders/{order}"),
            ] {
                let res = t.call(who, Method::GET, &path, None).await;
                assert_eq!(res.status, 200, "{}: {}", NAMES[who], res.json);
                assert!(res.json.get("buyer_contact").is_none(), "{}", res.json);
                let text = res.json.to_string();
                assert!(!text.contains("8077777777"), "{text}");
                assert!(!text.contains(common::BUYER_ADDRESS_LINE), "{text}");
            }
        }
        assert!(
            t.audit_actors("order.contact_revealed", order)
                .await
                .is_empty()
        );
    }
}

/// `require` maps a branch-tied staff row from the database onto its branch.
#[sqlx::test]
async fn require_maps_branch_staff_from_db(pool: PgPool) {
    let t = team(pool).await;
    let mut conn = t.app.pool.acquire().await.unwrap();
    let staff = t.ids[STAFF_A];
    for action in [
        Action::EditStock(t.branch_a),
        Action::ActOnOrder(t.branch_a),
    ] {
        let m = require(&mut conn, t.store_id, staff, action).await.unwrap();
        assert_eq!(m.role, Role::Staff);
        assert_eq!(m.branch_id, Some(t.branch_a));
    }
    for action in [
        Action::EditStock(t.branch_b),
        Action::ActOnOrder(t.branch_b),
    ] {
        let err = require(&mut conn, t.store_id, staff, action)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden("not_allowed")), "{err:?}");
    }
    let wide = require(
        &mut conn,
        t.store_id,
        t.ids[WIDE],
        Action::ActOnOrder(t.branch_b),
    )
    .await
    .unwrap();
    assert_eq!(wide.branch_id, None);
}

/// Permissions are read on every request: removed between the list and the step
/// is a stranger at the step.
#[sqlx::test]
async fn removed_member_gets_404_on_next_step(pool: PgPool) {
    let t = team(pool).await;
    let order = t.paid_order("self").await;
    let res = t
        .call(
            WIDE,
            Method::GET,
            &format!("/stores/{}/orders", t.store_id),
            None,
        )
        .await;
    assert_eq!(
        res.json["items"].as_array().unwrap().len(),
        1,
        "{}",
        res.json
    );
    sqlx::query("UPDATE store_members SET removed_at = now(), removed_by = $1 WHERE user_id = $2")
        .bind(t.ids[OWNER])
        .bind(t.ids[WIDE])
        .execute(&t.app.pool)
        .await
        .unwrap();
    let res = t
        .call(
            WIDE,
            Method::POST,
            &format!("/orders/{order}/picked-up"),
            None,
        )
        .await;
    assert_eq!(res.status, 404, "{}", res.json);
}

/// A dispute and a review outlive their store: after it is deleted, the owner and
/// managers still answer the dispute and reply to the review (as order steps and
/// `/return-received` still work), staff still get 403 and outsiders 404.
#[sqlx::test]
async fn deleted_store_dispute_and_review_still_answered(pool: PgPool) {
    let t = team(pool).await;
    let first = t.dispute().await;
    let second = t.dispute().await;
    let (_, review) = t.review().await;
    sqlx::query("UPDATE stores SET deleted_at = now() WHERE id = $1")
        .bind(t.store_id)
        .execute(&t.app.pool)
        .await
        .unwrap();

    let answer = json!({ "body": "We packed it with care; please send it back." });
    t.check(
        Method::POST,
        &format!("/disputes/{first}/response"),
        Some(answer.clone()),
        &[
            (WIDE, 403),
            (STAFF_A, 403),
            (REMOVED, 404),
            (STRANGER, 404),
            (OWNER, 200),
        ],
    )
    .await;
    t.check(
        Method::POST,
        &format!("/disputes/{second}/response"),
        Some(answer),
        &[(STAFF_B, 403), (MANAGER, 200)],
    )
    .await;
    for (dispute, who) in [(first, OWNER), (second, MANAGER)] {
        assert_eq!(
            t.audit_actors("dispute.seller_responded", dispute).await,
            vec![Some(t.ids[who])]
        );
    }
    t.check(
        Method::POST,
        &format!("/reviews/{review}/reply"),
        Some(json!({ "body": "Thank you" })),
        &[
            (STAFF_A, 403),
            (REMOVED, 404),
            (STRANGER, 404),
            (OWNER, 200),
        ],
    )
    .await;
}
