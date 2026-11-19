//! Dashboard order search and detail with the ledger (plan 6, task 4).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn get(app: &TestApp, path: &str, token: &str) -> TestResponse {
    app.request(Method::GET, path, None, Some(token)).await
}

async fn post(app: &TestApp, path: &str, body: Value, token: &str) -> TestResponse {
    app.request(Method::POST, path, Some(body), Some(token))
        .await
}

fn ids(v: &Value) -> Vec<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["order_id"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test]
async fn staff_other_than_the_owner_cannot_reach_orders(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let support = app.make_support().await;
    let o = app.shipped_order("self", "delivered").await;
    assert_eq!(get(&app, "/admin/orders", &support).await.status, 403);
    let path = format!("/admin/orders/{}", o.order_id);
    assert_eq!(get(&app, &path, &support).await.status, 403);
    assert_eq!(get(&app, &path, &o.buyer_token).await.status, 403);
}

#[sqlx::test]
async fn search_filters_and_pages(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let a = app.shipped_order("self", "delivered").await;
    let b = app.shipped_order("self", "picked_up").await;
    assert_ne!(a.store_id, b.store_id);

    let all = get(&app, "/admin/orders", &owner).await;
    assert_eq!(all.status, 200, "{}", all.json);
    assert_eq!(ids(&all.json).len(), 2);

    let delivered = get(&app, "/admin/orders?status=delivered", &owner).await;
    assert_eq!(ids(&delivered.json), vec![a.order_id.to_string()]);

    let by_store = get(
        &app,
        &format!("/admin/orders?store_id={}", a.store_id),
        &owner,
    )
    .await;
    assert_eq!(ids(&by_store.json), vec![a.order_id.to_string()]);
    let other = get(
        &app,
        &format!("/admin/orders?store_id={}", uuid::Uuid::new_v4()),
        &owner,
    )
    .await;
    assert!(ids(&other.json).is_empty());

    let future = get(&app, "/admin/orders?from=2999-01-01T00:00:00Z", &owner).await;
    assert!(ids(&future.json).is_empty());
    let past = get(&app, "/admin/orders?to=2999-01-01T00:00:00Z", &owner).await;
    assert_eq!(ids(&past.json).len(), 2);

    let first = get(&app, "/admin/orders?limit=1", &owner).await;
    assert_eq!(ids(&first.json).len(), 1);
    let cursor = first.json["next_cursor"].as_str().unwrap().to_string();
    let second = get(
        &app,
        &format!("/admin/orders?limit=1&before={cursor}"),
        &owner,
    )
    .await;
    assert_eq!(ids(&second.json).len(), 1);
    assert_ne!(ids(&first.json), ids(&second.json));
    assert!(second.json["next_cursor"].is_null());

    assert_eq!(
        get(&app, "/admin/orders?status=nope", &owner).await.status,
        422
    );
    assert_eq!(
        get(&app, "/admin/orders?before=nope", &owner).await.status,
        422
    );
    assert_eq!(
        get(&app, "/admin/orders?from=yesterday", &owner)
            .await
            .status,
        422
    );
}

#[sqlx::test]
async fn the_list_carries_no_buyer_data(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("self", "delivered").await;
    let text = get(&app, "/admin/orders", &owner).await.json.to_string();
    assert!(!text.contains(&o.buyer_id.to_string()), "{text}");
    assert!(!text.contains("address"), "{text}");
}

#[sqlx::test]
async fn detail_shows_the_order_and_a_balanced_ledger(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("self", "delivered").await;

    let rel = post(
        &app,
        &format!("/admin/orders/{}/release", o.order_id),
        json!({"note": "checked"}),
        &owner,
    )
    .await;
    assert_eq!(rel.status, 200, "{}", rel.json);

    let res = get(&app, &format!("/admin/orders/{}", o.order_id), &owner).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let d = &res.json;
    assert_eq!(d["order"]["order_id"], o.order_id.to_string());
    assert_eq!(d["order"]["status"], "completed");
    let unit = d["items"][0]["unit_price_kobo"].as_i64().unwrap();
    let qty = d["items"][0]["quantity"].as_i64().unwrap();
    assert_eq!(d["order"]["items_total_kobo"].as_i64().unwrap(), unit * qty);
    assert!(d["payment"]["reference"].is_string());
    assert!(d["payout"]["status"].is_string());
    assert!(d["refund"].is_null());
    assert!(d["dispute"].is_null());
    assert!(!d["delivery_events"].as_array().unwrap().is_empty());

    // The ledger: every posting balances for this order, and so does the whole.
    let postings = d["ledger"]["postings"].as_array().unwrap();
    let kinds: Vec<&str> = postings
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"release"), "{kinds:?}");
    for p in postings {
        assert_eq!(p["sum_kobo"], 0, "{p}");
        assert!(!p["entries"].as_array().unwrap().is_empty());
    }
    assert_eq!(d["ledger"]["balance_kobo"], 0);
    let escrow = d["ledger"]["balances"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["account"] == "escrow")
        .map(|b| b["kobo"].as_i64().unwrap());
    assert_eq!(escrow, Some(0), "escrow is empty after release");
}

#[sqlx::test]
async fn detail_of_a_refunded_order_shows_the_refund_posting(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("self", "delivered").await;
    let held = post(
        &app,
        &format!("/admin/orders/{}/hold", o.order_id),
        json!({"reason": "buyer complaint"}),
        &owner,
    )
    .await;
    assert_eq!(held.status, 200, "{}", held.json);
    let res = post(
        &app,
        &format!("/admin/orders/{}/refund", o.order_id),
        json!({"note": "damaged"}),
        &owner,
    )
    .await;
    assert_eq!(res.status, 200, "{}", res.json);

    let d = get(&app, &format!("/admin/orders/{}", o.order_id), &owner)
        .await
        .json;
    assert_eq!(d["refund"]["kind"], "full");
    assert!(d["refund"]["amount_kobo"].as_i64().unwrap() > 0);
    let kinds: Vec<&str> = d["ledger"]["postings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"refund"), "{kinds:?}");
    assert_eq!(d["ledger"]["balance_kobo"], 0);
}

#[sqlx::test]
async fn detail_names_an_open_dispute(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("self", "delivered").await;
    let opened = app
        .request(
            Method::POST,
            &format!("/orders/{}/dispute", o.order_id),
            Some(json!({
                "reason": "damaged",
                "body": "The zip was torn and the lining was stained when it arrived",
                "photo_keys": []
            })),
            Some(&o.buyer_token),
        )
        .await;
    assert_eq!(opened.status, 201, "{}", opened.json);

    let d = get(&app, &format!("/admin/orders/{}", o.order_id), &owner)
        .await
        .json;
    assert_eq!(d["dispute"]["id"], opened.json["id"]);
    assert_eq!(d["dispute"]["status"], "awaiting_seller");
    assert_eq!(d["order"]["held"], true);
}

#[sqlx::test]
async fn detail_is_audited_and_unknown_ids_are_404(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("self", "delivered").await;
    get(&app, &format!("/admin/orders/{}", o.order_id), &owner).await;
    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_log WHERE action = 'admin.viewed' \
         AND detail->>'view' = 'order_detail'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    assert_eq!(
        get(
            &app,
            &format!("/admin/orders/{}", uuid::Uuid::new_v4()),
            &owner
        )
        .await
        .status,
        404
    );
    assert_eq!(get(&app, "/admin/orders/nope", &owner).await.status, 404);
}

#[sqlx::test]
async fn detail_of_a_partial_refund_shows_both_posting_keys(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let owner = app.owner_token().await;
    let o = app.shipped_order("self", "delivered").await;
    sqlx::query("UPDATE orders SET held_at = now(), hold_reason = 'dispute' WHERE id = $1")
        .bind(o.order_id)
        .execute(&pool)
        .await
        .unwrap();
    let settings = server::settings::load(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    server::escrow::partial::partial_refund_in(&mut tx, &settings, o.order_id, 4_000)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let d = get(&app, &format!("/admin/orders/{}", o.order_id), &owner)
        .await
        .json;
    assert_eq!(d["refund"]["kind"], "partial");
    assert_eq!(d["order"]["status"], "completed");
    let kinds: Vec<&str> = d["ledger"]["postings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"refund") && kinds.contains(&"release"),
        "{kinds:?}"
    );
    for p in d["ledger"]["postings"].as_array().unwrap() {
        assert_eq!(p["sum_kobo"], 0, "{p}");
    }
    assert_eq!(d["ledger"]["balance_kobo"], 0);
}
