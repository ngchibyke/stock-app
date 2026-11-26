//! A store's branches (part 7e-1, task 6): the first branch made with the store, the
//! owner adding, editing and closing branches, and what closing does to stock,
//! invites and the staff tied to the branch.

mod common;

use axum::http::Method;
use common::{Listing, TestApp, TestResponse};
use serde_json::{Value, json};
use server::{error::AppError, stock};
use sqlx::PgPool;
use uuid::Uuid;

const OWNER_PHONE: &str = "08071000001";
const MANAGER_PHONE: &str = "08073000001";
const STAFF_PHONE: &str = "08073000002";
const OTHER_PHONE: &str = "08073000003";
const BUYER_PHONE: &str = "08072000001";

async fn shop(app: &TestApp, stock: i32) -> Listing {
    app.make_active_store_with_product(OWNER_PHONE, "Branch Shop", 1_000, stock)
        .await
}

fn branch_body(name: &str) -> Value {
    json!({
        "name": name,
        "pickup_address": "4 Ahmadu Bello Way, Garki",
        "pickup_area_council": "amac",
        "business_phone": "08035550000",
        "self_delivery_fee_kobo": 0,
    })
}

async fn add_branch(app: &TestApp, token: &str, store: Uuid, body: Value) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/stores/{store}/branches"),
        Some(body),
        Some(token),
    )
    .await
}

async fn added(app: &TestApp, token: &str, store: Uuid, name: &str) -> Uuid {
    let res = add_branch(app, token, store, branch_body(name)).await;
    assert_eq!(res.status, 201, "{}", res.json);
    res.json["id"].as_str().unwrap().parse().unwrap()
}

async fn list(app: &TestApp, token: &str, store: Uuid) -> TestResponse {
    app.request(
        Method::GET,
        &format!("/stores/{store}/branches"),
        None,
        Some(token),
    )
    .await
}

async fn patch_branch(
    app: &TestApp,
    token: &str,
    store: Uuid,
    branch: Uuid,
    body: Value,
) -> TestResponse {
    app.request(
        Method::PATCH,
        &format!("/stores/{store}/branches/{branch}"),
        Some(body),
        Some(token),
    )
    .await
}

async fn close(app: &TestApp, token: &str, store: Uuid, branch: Uuid) -> TestResponse {
    app.request(
        Method::POST,
        &format!("/stores/{store}/branches/{branch}/close"),
        None,
        Some(token),
    )
    .await
}

fn code_of(res: &TestResponse) -> &str {
    res.json["error"]["code"].as_str().unwrap_or("")
}

async fn first_branch(app: &TestApp, store: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM store_branches WHERE store_id = $1 AND position = 0")
        .bind(store)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn audits(app: &TestApp, action: &str, store: Uuid) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT detail FROM audit_log WHERE action = $1 AND subject_id = $2 ORDER BY id",
    )
    .bind(action)
    .bind(store)
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

/// The owner invites `phone` (optionally tied to `branch`); the invitee accepts.
async fn join(
    app: &TestApp,
    owner: &str,
    store: Uuid,
    phone: &str,
    role: &str,
    branch: Option<Uuid>,
) -> (Uuid, String) {
    let (id, token) = app.sign_up_cleared(phone).await;
    let invite = send_invite(app, owner, store, phone, role, branch).await;
    let res = app
        .request(
            Method::POST,
            &format!("/me/invites/{invite}/accept"),
            None,
            Some(&token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    (id, token)
}

async fn send_invite(
    app: &TestApp,
    owner: &str,
    store: Uuid,
    phone: &str,
    role: &str,
    branch: Option<Uuid>,
) -> Uuid {
    let res = app
        .request(
            Method::POST,
            &format!("/stores/{store}/invites"),
            Some(json!({ "phone": phone, "role": role, "branch_id": branch })),
            Some(owner),
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    res.json["id"].as_str().unwrap().parse().unwrap()
}

/// Each variant's totals equal the sum over its store's open branches.
async fn assert_totals_match_open_branches(pool: &PgPool, variant: Uuid) {
    let (stock, reserved, sum_stock, sum_reserved): (i32, i32, i64, i64) = sqlx::query_as(
        "SELECT v.stock, v.reserved_quantity, \
                COALESCE(sum(bs.stock) FILTER (WHERE b.closed_at IS NULL), 0)::bigint, \
                COALESCE(sum(bs.reserved_quantity) FILTER (WHERE b.closed_at IS NULL), 0)::bigint \
         FROM product_variants v \
         LEFT JOIN branch_stock bs ON bs.variant_id = v.id \
         LEFT JOIN store_branches b ON b.id = bs.branch_id \
         WHERE v.id = $1 GROUP BY v.id",
    )
    .bind(variant)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        (i64::from(stock), i64::from(reserved)),
        (sum_stock, sum_reserved)
    );
}

#[sqlx::test]
async fn create_store_makes_first_branch(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 3).await;
    let res = list(&app, &l.seller_token, l.store_id).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let branches = res.json.as_array().unwrap();
    assert_eq!(branches.len(), 1, "{}", res.json);
    let b = &branches[0];
    assert_eq!(b["name"], "Branch Shop");
    assert_eq!(b["position"], 0);
    assert_eq!(b["pickup_address"], "12 Unique Street, Wuse 2");
    assert_eq!(b["pickup_area_council"], "amac");
    assert_eq!(b["self_delivery_fee_kobo"], 0);
    assert_eq!(b["collect_enabled"], false);
    assert!(b["closed_at"].is_null());
    assert!(b["business_phone"].as_str().unwrap().ends_with("1234567"));
    // The courier's address code is a cache, never shown.
    assert!(b.get("pickup_courier_address_code").is_none());
    // The store's stock sits at that branch.
    let at: i32 = sqlx::query_scalar("SELECT stock FROM branch_stock WHERE branch_id = $1")
        .bind(first_branch(&app, l.store_id).await)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(at, 3);
}

#[sqlx::test]
async fn patch_store_pickup_edits_first_branch(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 1).await;
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{}", l.store_id),
            Some(json!({
                "pickup_address": "9 New Road, Maitama",
                "pickup_area_council": "bwari",
                "pickup_postcode": "FC01ABC1234",
                "business_phone": "08039998888",
                "self_delivery_fee_kobo": 50_000,
            })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = list(&app, &l.seller_token, l.store_id).await;
    let b = &res.json[0];
    assert_eq!(b["pickup_address"], "9 New Road, Maitama");
    assert_eq!(b["pickup_area_council"], "bwari");
    assert_eq!(b["pickup_postcode"], "FC01ABC1234");
    assert!(b["business_phone"].as_str().unwrap().ends_with("9998888"));
    assert_eq!(b["self_delivery_fee_kobo"], 50_000);
}

#[sqlx::test]
async fn store_rename_renames_auto_named_first_branch(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 1).await;
    let rename = |name: &'static str| {
        let (app, l) = (&app, &l);
        async move {
            let res = app
                .request(
                    Method::PATCH,
                    &format!("/stores/{}", l.store_id),
                    Some(json!({ "name": name })),
                    Some(&l.seller_token),
                )
                .await;
            assert_eq!(res.status, 200, "{}", res.json);
        }
    };
    rename("Corner Shop").await;
    let res = list(&app, &l.seller_token, l.store_id).await;
    assert_eq!(res.json[0]["name"], "Corner Shop");

    // Once the owner names the branch, a store rename leaves it alone.
    let first = first_branch(&app, l.store_id).await;
    let res = patch_branch(
        &app,
        &l.seller_token,
        l.store_id,
        first,
        json!({ "name": "Wuse" }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    rename("Big Shop").await;
    let res = list(&app, &l.seller_token, l.store_id).await;
    assert_eq!(res.json[0]["name"], "Wuse");
}

#[sqlx::test]
async fn add_branch_and_cap(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 1).await;
    let (_, manager) = join(
        &app,
        &l.seller_token,
        l.store_id,
        MANAGER_PHONE,
        "manager",
        None,
    )
    .await;
    // A seller, so the seller gate lets them through to the store check.
    let (_, stranger) = app.make_seller(OTHER_PHONE).await;

    let res = add_branch(&app, &l.seller_token, l.store_id, branch_body("Garki")).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["name"], "Garki");
    assert_eq!(res.json["position"], 1);
    assert_eq!(res.json["pickup_address"], "4 Ahmadu Bello Way, Garki");
    assert_eq!(res.json["collect_enabled"], false);
    let garki: Uuid = res.json["id"].as_str().unwrap().parse().unwrap();
    // Ids only in the audit, never the address or phone.
    let detail = audits(&app, "store.branch_added", l.store_id).await;
    assert_eq!(detail, vec![json!({ "branch_id": garki })]);

    // Owner only; a manager may read the list but not add.
    let res = add_branch(&app, &manager, l.store_id, branch_body("Wuse 2")).await;
    assert_eq!((res.status, code_of(&res)), (403, "not_allowed"));
    let res = add_branch(&app, &stranger, l.store_id, branch_body("Wuse 2")).await;
    assert_eq!(res.status, 404);
    let res = list(&app, &stranger, l.store_id).await;
    assert_eq!(res.status, 404);
    let res = list(&app, &manager, l.store_id).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json[1]["pickup_address"], "4 Ahmadu Bello Way, Garki");

    // The store delivers by courier and itself: a branch needs a fee for self-delivery.
    let mut no_fee = branch_body("Kubwa");
    no_fee["self_delivery_fee_kobo"] = Value::Null;
    let res = add_branch(&app, &l.seller_token, l.store_id, no_fee).await;
    assert_eq!(
        (res.status, code_of(&res)),
        (422, "delivery_details_required")
    );
    let mut with_fee = branch_body("Kubwa");
    with_fee["self_delivery_fee_kobo"] = json!(20_000);
    with_fee["collect_enabled"] = json!(true);
    let res = add_branch(&app, &l.seller_token, l.store_id, with_fee).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["collect_enabled"], true);

    // 20 open branches at most.
    for n in 3..20 {
        added(&app, &l.seller_token, l.store_id, &format!("Branch {n}")).await;
    }
    let res = add_branch(
        &app,
        &l.seller_token,
        l.store_id,
        branch_body("One too many"),
    )
    .await;
    assert_eq!((res.status, code_of(&res)), (409, "branch_limit"));

    // Closing one frees a place; positions are never reused.
    let res = close(&app, &l.seller_token, l.store_id, garki).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = add_branch(&app, &l.seller_token, l.store_id, branch_body("Again")).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["position"], 20);
}

#[sqlx::test]
async fn close_refused_with_open_orders_and_for_last_branch(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let first = first_branch(&app, l.store_id).await;
    let garki = added(&app, &l.seller_token, l.store_id, "Garki").await;

    // A pending checkout is an open order. Garki holds no stock, so checkout sends
    // from the first branch; the order is moved to Garki by hand.
    let (_, buyer) = app.sign_up_verified(BUYER_PHONE).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    let res = app.checkout(&buyer, "mono_directpay").await;
    assert_eq!(res.status, 201, "{}", res.json);
    sqlx::query("UPDATE orders SET branch_id = $1 WHERE store_id = $2")
        .bind(garki)
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();

    let res = close(&app, &l.seller_token, l.store_id, garki).await;
    assert_eq!((res.status, code_of(&res)), (409, "branch_has_open_orders"));
    let open: bool =
        sqlx::query_scalar("SELECT closed_at IS NULL FROM store_branches WHERE id = $1")
            .bind(garki)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(open);

    // Once the order is cancelled (its unit given back), Garki closes, and closing it
    // again is refused.
    sqlx::query("UPDATE orders SET status = 'cancelled' WHERE store_id = $1")
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    stock::apply(&mut tx, first, l.variant_id, 0, -1)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let res = close(&app, &l.seller_token, l.store_id, garki).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(res.json["closed_at"].is_string());
    let detail = audits(&app, "store.branch_closed", l.store_id).await;
    assert_eq!(detail, vec![json!({ "branch_id": garki })]);
    let res = close(&app, &l.seller_token, l.store_id, garki).await;
    assert_eq!((res.status, code_of(&res)), (409, "branch_closed"));
    let res = patch_branch(
        &app,
        &l.seller_token,
        l.store_id,
        garki,
        json!({ "name": "x" }),
    )
    .await;
    assert_eq!((res.status, code_of(&res)), (409, "branch_closed"));

    // The last open branch cannot close.
    let res = close(&app, &l.seller_token, l.store_id, first).await;
    assert_eq!((res.status, code_of(&res)), (409, "last_branch"));

    // A branch of another store is not found.
    let other = app
        .make_active_store_with_product("08071000002", "Other Shop", 1_000, 1)
        .await;
    let res = close(&app, &l.seller_token, other.store_id, garki).await;
    assert_eq!(res.status, 404);
    let res = close(&app, &other.seller_token, other.store_id, garki).await;
    assert_eq!(res.status, 404);
}

#[sqlx::test]
async fn first_branch_closes_and_checkout_routes_to_the_rest(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 1).await;
    let first = first_branch(&app, l.store_id).await;
    let garki = added(&app, &l.seller_token, l.store_id, "Garki").await;
    let res = app
        .request(
            Method::PUT,
            &format!("/branches/{garki}/stock"),
            Some(json!({ "items": [{ "variant_id": l.variant_id, "stock": 4 }] })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);

    // The first branch is no longer special: it closes like any other.
    let res = close(&app, &l.seller_token, l.store_id, first).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let detail = audits(&app, "store.branch_closed", l.store_id).await;
    assert_eq!(detail, vec![json!({ "branch_id": first })]);
    // The owner's view now shows the first open branch's pickup.
    let res = app
        .request(Method::GET, "/stores/mine", None, Some(&l.seller_token))
        .await;
    assert_eq!(res.json[0]["pickup_address"], "4 Ahmadu Bello Way, Garki");

    // Checkout sends from Garki.
    let (_, buyer) = app.sign_up_verified(BUYER_PHONE).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 2).await.status, 200);
    let res = app.checkout(&buyer, "mono_directpay").await;
    assert_eq!(res.status, 201, "{}", res.json);
    let branch: Uuid = sqlx::query_scalar("SELECT branch_id FROM orders WHERE store_id = $1")
        .bind(l.store_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(branch, garki);
    let held: i32 = sqlx::query_scalar(
        "SELECT reserved_quantity FROM branch_stock WHERE branch_id = $1 AND variant_id = $2",
    )
    .bind(garki)
    .bind(l.variant_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(held, 2);

    // Garki is now the last open branch.
    let res = close(&app, &l.seller_token, l.store_id, garki).await;
    assert_eq!((res.status, code_of(&res)), (409, "last_branch"));
}

#[sqlx::test]
async fn closed_branch_stock_not_sold(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 5).await;
    let garki = added(&app, &l.seller_token, l.store_id, "Garki").await;
    let res = app
        .request(
            Method::PUT,
            &format!("/branches/{garki}/stock"),
            Some(json!({ "items": [{ "variant_id": l.variant_id, "stock": 8 }] })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    // Two units held at Garki by a checkout that has not yet finished.
    let mut tx = app.pool.begin().await.unwrap();
    stock::apply(&mut tx, garki, l.variant_id, 0, 2)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let product = format!("/products/{}", l.product_id);
    let res = app.request(Method::GET, &product, None, None).await;
    assert_eq!(res.json["variants"][0]["available_stock"], 6);
    assert_eq!(
        (
            app.stock(l.variant_id).await,
            app.reserved(l.variant_id).await
        ),
        (13, 2)
    );

    let res = close(&app, &l.seller_token, l.store_id, garki).await;
    assert_eq!(res.status, 200, "{}", res.json);

    // Only the first branch's 5 are for sale now.
    let res = app.request(Method::GET, &product, None, None).await;
    assert_eq!(res.json["variants"][0]["available_stock"], 5);
    assert_eq!(
        (
            app.stock(l.variant_id).await,
            app.reserved(l.variant_id).await
        ),
        (5, 0)
    );
    assert_totals_match_open_branches(&app.pool, l.variant_id).await;

    // No new reservation at the closed branch.
    let mut tx = app.pool.begin().await.unwrap();
    let err = stock::apply(&mut tx, garki, l.variant_id, 0, 1)
        .await
        .unwrap_err();
    assert!(
        matches!(err, AppError::ConflictDetail("out_of_stock", _)),
        "{err:?}"
    );
    drop(tx);
    let mut tx = app.pool.begin().await.unwrap();
    assert_eq!(
        stock::lock_available(&mut tx, garki, l.variant_id)
            .await
            .unwrap(),
        0
    );
    drop(tx);
    // Of the pending checkouts' two units, one is sold and one goes back: both move
    // Garki's own row only.
    let mut tx = app.pool.begin().await.unwrap();
    stock::apply(&mut tx, garki, l.variant_id, -1, -1)
        .await
        .unwrap();
    stock::apply(&mut tx, garki, l.variant_id, 0, -1)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let at: (i32, i32) =
        sqlx::query_as("SELECT stock, reserved_quantity FROM branch_stock WHERE branch_id = $1")
            .bind(garki)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(at, (7, 0));
    assert_eq!(
        (
            app.stock(l.variant_id).await,
            app.reserved(l.variant_id).await
        ),
        (5, 0)
    );
    assert_totals_match_open_branches(&app.pool, l.variant_id).await;

    // The seller cannot restock a closed branch.
    let res = app
        .request(
            Method::PUT,
            &format!("/branches/{garki}/stock"),
            Some(json!({ "items": [{ "variant_id": l.variant_id, "stock": 9 }] })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 404);
}

#[sqlx::test]
async fn staff_tied_to_closed_branch_loses_access(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 1).await;
    let garki = added(&app, &l.seller_token, l.store_id, "Garki").await;
    let (staff_id, staff) = join(
        &app,
        &l.seller_token,
        l.store_id,
        STAFF_PHONE,
        "staff",
        Some(garki),
    )
    .await;
    let pending = send_invite(
        &app,
        &l.seller_token,
        l.store_id,
        OTHER_PHONE,
        "staff",
        Some(garki),
    )
    .await;
    let elsewhere = send_invite(
        &app,
        &l.seller_token,
        l.store_id,
        MANAGER_PHONE,
        "manager",
        None,
    )
    .await;

    // Staff see the branches' names and area councils, not addresses or phones.
    let res = list(&app, &staff, l.store_id).await;
    assert_eq!(res.status, 200, "{}", res.json);
    for b in res.json.as_array().unwrap() {
        assert!(b["name"].is_string());
        assert_eq!(b["pickup_area_council"], "amac");
        for hidden in ["pickup_address", "business_phone", "pickup_postcode"] {
            assert!(b.get(hidden).is_none(), "{hidden} shown to staff: {b}");
        }
    }
    // Nor does their list of stores carry the store's pickup address or phone.
    let res = app
        .request(Method::GET, "/stores/mine", None, Some(&staff))
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let mine = &res.json[0];
    assert_eq!(mine["id"], json!(l.store_id));
    assert_eq!(mine["my_branch_id"], json!(garki));
    assert_eq!(mine["pickup_area_council"], "amac");
    for hidden in ["pickup_address", "business_phone", "pickup_postcode"] {
        assert!(
            mine.get(hidden).is_none(),
            "{hidden} shown to staff: {mine}"
        );
    }

    // An order fulfilled at Garki, finished. Checkout still sends from the first
    // branch (until task 7), so the order is moved there by hand and its units given back.
    let first = first_branch(&app, l.store_id).await;
    let (_, buyer) = app.sign_up_verified(BUYER_PHONE).await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);
    assert_eq!(app.checkout(&buyer, "mono_directpay").await.status, 201);
    let order: Uuid = sqlx::query_scalar(
        "UPDATE orders SET branch_id = $1, status = 'cancelled' WHERE store_id = $2 RETURNING id",
    )
    .bind(garki)
    .bind(l.store_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    stock::apply(&mut tx, first, l.variant_id, 0, -1)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let (app, staff) = (&app, staff.as_str());
    let see_order = || async move {
        app.request(Method::GET, &format!("/orders/{order}"), None, Some(staff))
            .await
    };
    let store_orders = || async move {
        app.request(
            Method::GET,
            &format!("/stores/{}/orders", l.store_id),
            None,
            Some(staff),
        )
        .await
    };
    let stock_at = |branch: Uuid, n: i32| async move {
        app.request(
            Method::PUT,
            &format!("/branches/{branch}/stock"),
            Some(json!({ "items": [{ "variant_id": l.variant_id, "stock": n }] })),
            Some(staff),
        )
        .await
    };

    // They keep their branch's stock and orders until it closes.
    let res = stock_at(garki, 2).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(see_order().await.status, 200);
    let res = store_orders().await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(
        res.json.to_string().contains(&order.to_string()),
        "{}",
        res.json
    );

    let res = close(app, &l.seller_token, l.store_id, garki).await;
    assert_eq!(res.status, 200, "{}", res.json);

    // Still in the team, but with no branch's orders or stock.
    let res = list(app, staff, l.store_id).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(stock_at(garki, 1).await.status, 404);
    let res = stock_at(first, 1).await;
    assert_eq!((res.status, code_of(&res)), (403, "not_allowed"));
    assert_eq!(see_order().await.status, 404);
    let res = store_orders().await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert!(
        !res.json.to_string().contains(&order.to_string()),
        "{}",
        res.json
    );
    let member: Option<Uuid> = sqlx::query_scalar(
        "SELECT branch_id FROM store_members WHERE user_id = $1 AND removed_at IS NULL",
    )
    .bind(staff_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(member, Some(garki));

    // Moved to the first branch, they work there.
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{}/members/{staff_id}", l.store_id),
            Some(json!({ "branch_id": first })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    let res = stock_at(first, 1).await;
    assert_eq!(res.status, 200, "{}", res.json);

    // The invite tied to the branch is revoked; the store-wide one stays open.
    let revoked: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM store_invites WHERE store_id = $1 AND revoked_at IS NOT NULL",
    )
    .bind(l.store_id)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(revoked, vec![pending]);
    assert_ne!(pending, elsewhere);
    let detail = audits(app, "store.invite_revoked", l.store_id).await;
    assert_eq!(
        detail,
        vec![json!({ "invite_id": pending, "reason": "branch_closed" })]
    );
}

#[sqlx::test]
async fn moved_branch_resets_courier_code(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = shop(&app, 1).await;
    let garki = added(&app, &l.seller_token, l.store_id, "Garki").await;
    let code = |branch: Uuid| {
        let pool = app.pool.clone();
        async move {
            sqlx::query_scalar::<_, Option<i64>>(
                "SELECT pickup_courier_address_code FROM store_branches WHERE id = $1",
            )
            .bind(branch)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let set_code = |branch: Uuid| {
        let pool = app.pool.clone();
        async move {
            sqlx::query(
                "UPDATE store_branches SET pickup_courier_address_code = 4242 WHERE id = $1",
            )
            .bind(branch)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    set_code(garki).await;

    // A rename, a fee or turning collect on does not move the branch.
    let res = patch_branch(
        &app,
        &l.seller_token,
        l.store_id,
        garki,
        json!({ "name": "Garki 2", "self_delivery_fee_kobo": 10_000, "collect_enabled": true }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["name"], "Garki 2");
    assert_eq!(res.json["collect_enabled"], true);
    assert_eq!(code(garki).await, Some(4242));
    // The same address again is no move either.
    let res = patch_branch(
        &app,
        &l.seller_token,
        l.store_id,
        garki,
        json!({ "pickup_address": "4 Ahmadu Bello Way, Garki" }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(code(garki).await, Some(4242));

    let res = patch_branch(
        &app,
        &l.seller_token,
        l.store_id,
        garki,
        json!({ "pickup_address": "7 Moved Street, Garki" }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["pickup_address"], "7 Moved Street, Garki");
    assert_eq!(code(garki).await, None);

    // A new phone for the courier to call resets it too.
    set_code(garki).await;
    let res = patch_branch(
        &app,
        &l.seller_token,
        l.store_id,
        garki,
        json!({ "business_phone": "08036667777" }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(code(garki).await, None);
    let detail = audits(&app, "store.branch_updated", l.store_id).await;
    assert_eq!(
        detail.last().unwrap(),
        &json!({ "branch_id": garki, "fields_changed": ["business_phone"] })
    );

    // Moving the first branch stamps that branch only: the store row is no longer written.
    let first = first_branch(&app, l.store_id).await;
    set_code(first).await;
    let res = patch_branch(
        &app,
        &l.seller_token,
        l.store_id,
        first,
        json!({ "pickup_address": "1 First Avenue, Wuse" }),
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(code(first).await, None);
    let moved: (bool, bool) = sqlx::query_as(
        "SELECT b.pickup_moved_at IS NOT NULL, s.pickup_moved_at IS NOT NULL \
         FROM store_branches b JOIN stores s ON s.id = b.store_id WHERE b.id = $1",
    )
    .bind(first)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(moved, (true, false));
    let res = app
        .request(Method::GET, "/stores/mine", None, Some(&l.seller_token))
        .await;
    assert_eq!(res.json[0]["pickup_address"], "1 First Avenue, Wuse");
}
