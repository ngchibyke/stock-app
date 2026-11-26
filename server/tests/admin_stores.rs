//! Dashboard store list, view and suspension (plan 6, task 3).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn get(app: &TestApp, path: &str, token: &str) -> TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

async fn post(app: &TestApp, path: &str, body: Option<Value>, token: &str) -> TestResponse {
    app.request(Method::POST, path, body, Some(token)).await
}

fn ids(v: &Value) -> Vec<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

async fn status(pool: &PgPool, store: uuid::Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM stores WHERE id = $1")
        .bind(store)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn owner_lists_searches_and_views_stores(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let a = app
        .make_active_store_with_product("08031110001", "Alpha Threads", 500_000, 5)
        .await;
    let b = app
        .make_active_store_with_product("08031110002", "Beta Bags", 500_000, 5)
        .await;

    let all = get(&app, "/admin/stores", &owner).await;
    assert_eq!(all.status, 200, "{}", all.json);
    assert_eq!(ids(&all.json).len(), 2);

    let named = get(&app, "/admin/stores?q=alpha", &owner).await;
    assert_eq!(ids(&named.json), vec![a.store_id.to_string()]);

    let by_status = get(&app, "/admin/stores?status=active", &owner).await;
    assert_eq!(ids(&by_status.json).len(), 2);
    assert_eq!(
        get(&app, "/admin/stores?status=nope", &owner).await.status,
        422
    );

    let one = get(&app, &format!("/admin/stores/{}", b.store_id), &owner).await;
    assert_eq!(one.status, 200, "{}", one.json);
    assert_eq!(one.json["name"], "Beta Bags");
    assert_eq!(one.json["owner_id"], b.seller_id.to_string());
    assert_eq!(one.json["product_count"], 1);
    assert_eq!(one.json["open_order_count"], 0);
    assert_eq!(one.json["payout_account_status"], "verified");
    assert_eq!(one.json["suspended"], false);
    assert_eq!(get(&app, "/admin/stores/nope", &owner).await.status, 404);
}

#[sqlx::test]
async fn only_the_owner_reaches_store_routes(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let support = app.make_support().await;
    let l = app
        .make_active_store_with_product("08031110001", "Alpha", 500_000, 5)
        .await;
    assert_eq!(get(&app, "/admin/stores", &support).await.status, 403);
    let path = format!("/admin/stores/{}/suspend", l.store_id);
    let res = post(&app, &path, Some(json!({"reason": "x"})), &support).await;
    assert_eq!(res.status, 403);
    assert_eq!(
        post(&app, &path, Some(json!({"reason": "x"})), &l.seller_token)
            .await
            .status,
        403
    );
}

#[sqlx::test]
async fn suspending_hides_the_store_and_blocks_checkout_but_keeps_paid_orders(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    // A paid order already running on this store.
    let o = app.shipped_order("self", "picked_up").await;
    let l = app
        .make_active_store_with_product("08031110009", "Other Shop", 500_000, 5)
        .await;

    let res = post(
        &app,
        &format!("/admin/stores/{}/suspend", o.store_id),
        Some(json!({"reason": "Repeated complaints"})),
        &owner,
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["status"], "suspended");
    assert_eq!(status(&pool, o.store_id).await, "suspended");

    let view = get(&app, &format!("/admin/stores/{}", o.store_id), &owner).await;
    assert_eq!(view.json["suspended"], true);
    assert_eq!(view.json["suspended_reason"], "Repeated complaints");
    assert_eq!(view.json["open_order_count"], 1);

    // The public store page is hidden from a buyer.
    let (_, buyer) = app.sign_up_verified("08031110010").await;
    let slug: String = sqlx::query_scalar("SELECT slug FROM stores WHERE id = $1")
        .bind(o.store_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        get(&app, &format!("/stores/{slug}"), &buyer).await.status,
        404
    );

    // The existing order carries on: the seller can still see it and the
    // buyer can still confirm it.
    let still: String = sqlx::query_scalar("SELECT status::text FROM orders WHERE id = $1")
        .bind(o.order_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(still, "picked_up");

    // Another store is unaffected.
    assert_eq!(status(&pool, l.store_id).await, "active");
}

#[sqlx::test]
async fn a_cart_line_from_a_suspended_store_is_unavailable_at_checkout(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let l = app
        .make_active_store_with_product("08031110001", "Alpha", 500_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified("08031110010").await;
    assert_eq!(app.set_cart(&buyer, l.variant_id, 1).await.status, 200);

    let res = post(
        &app,
        &format!("/admin/stores/{}/suspend", l.store_id),
        Some(json!({"reason": "Fraud report"})),
        &owner,
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);

    let cart = get(&app, "/cart", &buyer).await;
    assert_eq!(cart.json["items"][0]["available"], false);
    let out = app.checkout(&buyer, "mono_directpay").await;
    assert_eq!(out.status, 422, "{}", out.json);
    assert_eq!(out.json["error"]["code"], "item_unavailable");
}

#[sqlx::test]
async fn suspend_needs_a_reason_and_is_idempotent_by_conflict(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let l = app
        .make_active_store_with_product("08031110001", "Alpha", 500_000, 5)
        .await;
    let path = format!("/admin/stores/{}/suspend", l.store_id);

    assert_eq!(
        post(&app, &path, Some(json!({"reason": "  "})), &owner)
            .await
            .status,
        422
    );
    assert_eq!(
        post(
            &app,
            &path,
            Some(json!({"reason": "to 0123456785"})),
            &owner
        )
        .await
        .status,
        422
    );
    assert_eq!(
        post(&app, &path, Some(json!({"reason": "Fraud"})), &owner)
            .await
            .status,
        200
    );
    let again = post(&app, &path, Some(json!({"reason": "Fraud"})), &owner).await;
    assert_eq!(again.status, 409);
    assert_eq!(again.json["error"]["code"], "already_suspended");

    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_log WHERE action = 'admin.store_suspended' AND subject_id = $1",
    )
    .bind(l.store_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test]
async fn unsuspend_restores_active_or_pending_by_payout_account(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let l = app
        .make_active_store_with_product("08031110001", "Alpha", 500_000, 5)
        .await;
    let suspend = format!("/admin/stores/{}/suspend", l.store_id);
    let unsuspend = format!("/admin/stores/{}/unsuspend", l.store_id);

    // Not suspended yet.
    let early = post(&app, &unsuspend, None, &owner).await;
    assert_eq!(early.status, 409);
    assert_eq!(early.json["error"]["code"], "not_suspended");

    post(&app, &suspend, Some(json!({"reason": "Fraud"})), &owner).await;
    let back = post(&app, &unsuspend, None, &owner).await;
    assert_eq!(back.status, 200, "{}", back.json);
    assert_eq!(back.json["status"], "active");
    assert_eq!(back.json["suspended"], false);

    // A store whose payout account is no longer verified comes back pending.
    post(&app, &suspend, Some(json!({"reason": "Again"})), &owner).await;
    sqlx::query("UPDATE payout_accounts SET status = 'pending', verified_by = NULL, verified_at = NULL, pending_reason = 'name_mismatch' WHERE store_id = $1")
        .bind(l.store_id)
        .execute(&pool)
        .await
        .unwrap();
    let back = post(&app, &unsuspend, None, &owner).await;
    assert_eq!(back.json["status"], "pending");
}

#[sqlx::test]
async fn admin_detail_shows_the_plot_the_store_holds(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let placed = app
        .make_active_store_with_product("08031110001", "Placed", 500_000, 5)
        .await;
    let one = get(&app, &format!("/admin/stores/{}", placed.store_id), &owner).await;
    assert_eq!(one.json["plot"], "everything-else-1", "{}", one.json);

    sqlx::query("DELETE FROM store_plots WHERE store_id = $1")
        .bind(placed.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let one = get(&app, &format!("/admin/stores/{}", placed.store_id), &owner).await;
    assert!(one.json["plot"].is_null(), "{}", one.json);
    assert!(one.json.get("plot").is_some());
}

#[sqlx::test]
async fn admin_detail_lists_branches_and_team_without_addresses_or_phones(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let l = app
        .make_active_store_with_product("08031110021", "Branchy", 500_000, 5)
        .await;
    let addr = app.state.cipher.encrypt("9 Secret Road, Garki");
    let second: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO store_branches (store_id, name, pickup_address_enc, pickup_postcode, \
                                     collect_enabled, position, closed_at) \
         VALUES ($1, 'Garki', $2, 'FC12ABCDEFG', true, 1, now()) RETURNING id",
    )
    .bind(l.store_id)
    .bind(addr)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let (staff, _) = app.sign_up_verified("08031110022").await;
    let (gone, _) = app.sign_up_verified("08031110023").await;
    for (user, role, branch, removed) in [
        (staff, "staff", Some(second), false),
        (gone, "manager", None, true),
    ] {
        sqlx::query(
            "INSERT INTO store_members (store_id, user_id, role, branch_id, added_by, removed_at, removed_by) \
             VALUES ($1, $2, $3::store_member_role, $4, $5, CASE WHEN $6 THEN now() END, CASE WHEN $6 THEN $5 END)",
        )
        .bind(l.store_id)
        .bind(user)
        .bind(role)
        .bind(branch)
        .bind(l.seller_id)
        .bind(removed)
        .execute(&app.pool)
        .await
        .unwrap();
    }

    let res = get(&app, &format!("/admin/stores/{}", l.store_id), &owner).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let branches = res.json["branches"].as_array().unwrap();
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[1]["id"], second.to_string());
    assert_eq!(branches[1]["name"], "Garki");
    assert_eq!(branches[1]["collect_enabled"], true);
    assert!(branches[1]["closed_at"].is_string());
    assert!(branches[0]["closed_at"].is_null());
    assert!(branches[0].get("area_council").is_some());

    let team = res.json["team"].as_array().unwrap();
    assert_eq!(team.len(), 2);
    assert_eq!(team[0]["user_id"], staff.to_string());
    assert_eq!(team[0]["role"], "staff");
    assert_eq!(team[0]["branch_id"], second.to_string());
    assert!(team[0]["removed_at"].is_null());
    assert!(team[0]["display_name"].is_string());
    assert_eq!(team[1]["user_id"], gone.to_string());
    assert!(team[1]["removed_at"].is_string());

    let text = res.json.to_string();
    for banned in [
        "Secret",
        "Garki Road",
        "FC12ABCDEFG",
        "0803111",
        "pickup",
        "phone",
        "address",
        "postcode",
    ] {
        assert!(!text.contains(banned), "leaked {banned}: {text}");
    }
}
