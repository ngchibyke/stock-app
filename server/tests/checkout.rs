mod common;

use axum::http::Method;
use common::{Listing, TEST_REDIRECT_URL, TestApp};
use serde_json::json;
use server::limits::MAX_CHECKOUT_TOTAL_KOBO;
use sqlx::PgPool;
use uuid::Uuid;

const METHOD: &str = "mono_directpay";
const BUYER: &str = "08022222221";
const BUYER2: &str = "08022222222";

async fn listing(app: &TestApp, price: i64, stock: i32) -> Listing {
    app.make_active_store_with_product("08011111111", "Mama Put", price, stock)
        .await
}

async fn checkout_status(app: &TestApp, id: &str) -> String {
    sqlx::query_scalar("SELECT status::text FROM checkouts WHERE id = $1")
        .bind(id.parse::<Uuid>().unwrap())
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn count(app: &TestApp, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&app.pool).await.unwrap()
}

#[sqlx::test]
async fn checkout_splits_orders_per_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    // 1_050 * 5% = 52.5 -> 53 (half-up).
    let a = app
        .make_active_store_with_product("08011111111", "Mama Put", 1_050, 5)
        .await;
    let b = app
        .make_active_store_with_product("08011111112", "Bukka Hut", 1_000_000, 5)
        .await;
    let (buyer_id, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, a.variant_id, 1).await;
    app.set_cart(&buyer, b.variant_id, 2).await;

    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["total_kobo"], 2_001_050);
    let reference = res.json["reference"].as_str().unwrap();
    assert!(
        reference.starts_with("CK") && reference.len() == 22,
        "{reference}"
    );
    assert_eq!(
        res.json["payment_url"],
        format!("https://pay.fake.test/{reference}")
    );
    let orders = res.json["orders"].as_array().unwrap();
    assert_eq!(orders.len(), 2);
    let mut totals: Vec<(String, i64)> = orders
        .iter()
        .map(|o| {
            (
                o["store_name"].as_str().unwrap().to_string(),
                o["items_total_kobo"].as_i64().unwrap(),
            )
        })
        .collect();
    totals.sort();
    assert_eq!(
        totals,
        vec![("Bukka Hut".into(), 2_000_000), ("Mama Put".into(), 1_050)]
    );

    let rows: Vec<(Uuid, i64, i32, i64, i64, String, Uuid)> = sqlx::query_as(
        "SELECT store_id, items_total_kobo, platform_cut_bps, platform_cut_kobo, \
                delivery_fee_kobo, status::text, buyer_id FROM orders ORDER BY items_total_kobo",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                a.store_id,
                1_050,
                500,
                53,
                0,
                "pending_payment".into(),
                buyer_id
            ),
            (
                b.store_id,
                2_000_000,
                500,
                100_000,
                0,
                "pending_payment".into(),
                buyer_id
            ),
        ]
    );
    let sum: i64 = rows.iter().map(|r| r.1 + r.4).sum();
    assert_eq!(sum, 2_001_050);

    let (provider, payment_id, total, ttl_ok, method): (String, String, i64, bool, String) =
        sqlx::query_as(
            "SELECT provider, provider_payment_id, total_kobo, \
                    expires_at BETWEEN now() + interval '29 minutes' AND now() + interval '31 minutes', \
                    method FROM checkouts",
        )
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(provider, "fake");
    assert_eq!(payment_id, format!("fake_{reference}"));
    assert_eq!(total, 2_001_050);
    assert!(ttl_ok);
    assert_eq!(method, METHOD);

    // Items are snapshotted.
    let items: Vec<(Uuid, String, i64, i32)> = sqlx::query_as(
        "SELECT variant_id, product_name, unit_price_kobo, quantity FROM order_items ORDER BY unit_price_kobo",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        items,
        vec![
            (a.variant_id, "Ankara Shirt".into(), 1_050, 1),
            (b.variant_id, "Ankara Shirt".into(), 1_000_000, 2),
        ]
    );

    // The provider got the buyer's display name only, no email, and the configured return URL.
    let sent = app.fake.initiated();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].reference, reference);
    assert_eq!(sent[0].amount_kobo, 2_001_050);
    assert_eq!(sent[0].customer_name, "Ada");
    assert_eq!(sent[0].customer_email, None);
    assert_eq!(sent[0].redirect_url, TEST_REDIRECT_URL);

    // The cart stays until the checkout is paid (Task 6).
    let cart = app.request(Method::GET, "/cart", None, Some(&buyer)).await;
    assert_eq!(cart.json["items"].as_array().unwrap().len(), 2);
}

#[sqlx::test]
async fn checkout_reserves_stock(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(app.available(l.variant_id).await, 3);
    // Reserved, not sold: on-hand stock is unchanged until the payment settles.
    assert_eq!(app.stock(l.variant_id).await, 5);
    assert_eq!(app.reserved(l.variant_id).await, 2);
    // Buyers see available stock only; the seller also sees on hand and reserved.
    let path = format!("/products/{}", l.product_id);
    let public = app.request(Method::GET, &path, None, None).await;
    let v = &public.json["variants"][0];
    assert_eq!(v["available_stock"], 3);
    assert!(
        v.get("stock").is_none() && v.get("reserved_quantity").is_none(),
        "{v}"
    );
    let own = app
        .request(Method::GET, &path, None, Some(&l.seller_token))
        .await;
    let v = &own.json["variants"][0];
    assert_eq!(
        (
            v["stock"].as_i64(),
            v["reserved_quantity"].as_i64(),
            v["available_stock"].as_i64()
        ),
        (Some(5), Some(2), Some(3))
    );
}

#[sqlx::test]
async fn concurrent_last_unit_one_wins(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 1).await;
    let (_, b1) = app.sign_up_verified(BUYER).await;
    let (_, b2) = app.sign_up_verified(BUYER2).await;
    app.set_cart(&b1, l.variant_id, 1).await;
    app.set_cart(&b2, l.variant_id, 1).await;

    let (r1, r2) = tokio::join!(app.checkout(&b1, METHOD), app.checkout(&b2, METHOD));
    let mut statuses = [r1.status, r2.status];
    statuses.sort();
    assert_eq!(statuses, [201, 409], "{} / {}", r1.json, r2.json);
    let loser = if r1.status == 409 { &r1 } else { &r2 };
    assert_eq!(loser.json["error"]["code"], "out_of_stock");
    assert!(
        loser.json["error"]["message"]
            .as_str()
            .unwrap()
            .contains(&l.variant_id.to_string())
    );
    assert_eq!(app.available(l.variant_id).await, 0);
    assert_eq!(count(&app, "SELECT count(*) FROM checkouts").await, 1);
}

#[sqlx::test]
async fn unpublished_or_pending_store_item_unavailable(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;

    app.request(
        Method::PATCH,
        &format!("/products/{}", l.product_id),
        Some(json!({ "is_published": false })),
        Some(&l.seller_token),
    )
    .await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "item_unavailable");

    app.request(
        Method::PATCH,
        &format!("/products/{}", l.product_id),
        Some(json!({ "is_published": true })),
        Some(&l.seller_token),
    )
    .await;
    sqlx::query("UPDATE stores SET status = 'pending' WHERE id = $1")
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "item_unavailable");

    // Nothing was reserved or created.
    assert_eq!(app.available(l.variant_id).await, 5);
    assert_eq!(count(&app, "SELECT count(*) FROM checkouts").await, 0);
}

#[sqlx::test]
async fn disabled_method_rejected(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let res = app.checkout(&buyer, "paystack_card").await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "payment_method_disabled");
    assert_eq!(app.available(l.variant_id).await, 5);
    assert_eq!(app.fake.calls("initiate"), 0);
}

#[sqlx::test]
async fn own_store_item_rejected(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    // The cart refuses it (see tests/cart.rs), so put it there directly.
    sqlx::query("INSERT INTO cart_items (user_id, variant_id, quantity) VALUES ($1, $2, 1)")
        .bind(l.seller_id)
        .bind(l.variant_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = app.checkout(&l.seller_token, METHOD).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "own_store_item");
    assert_eq!(app.available(l.variant_id).await, 5);
}

#[sqlx::test]
async fn empty_cart_rejected(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "cart_empty");
    assert_eq!(
        app.request(
            Method::POST,
            "/checkout",
            Some(json!({"method": METHOD})),
            None
        )
        .await
        .status,
        401
    );
}

async fn assert_failed_and_restored(app: &TestApp, l: &Listing, res: &common::TestResponse) {
    assert_eq!(res.status, 502, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "payment_provider_unavailable");
    assert_eq!(app.available(l.variant_id).await, 5);
    assert_eq!(app.reserved(l.variant_id).await, 0);
    assert_eq!(app.stock(l.variant_id).await, 5);
    let (status, url): (String, Option<String>) =
        sqlx::query_as("SELECT status::text, payment_url FROM checkouts")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(status, "expired");
    assert_eq!(url, None);
    let orders: Vec<(String, bool)> =
        sqlx::query_as("SELECT status::text, cancelled_at IS NOT NULL FROM orders")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    assert_eq!(orders, vec![("cancelled".into(), true)]);
}

#[sqlx::test]
async fn provider_failure_restores_stock(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    app.fake.fail_initiate();
    let res = app.checkout(&buyer, METHOD).await;
    assert_failed_and_restored(&app, &l, &res).await;
}

#[sqlx::test]
async fn provider_timeout_restores_stock(pool: PgPool) {
    // Timeout: the provider may have created the payment, but nothing is paid until
    // verified; a later webhook goes through Task 6's paid-after-expiry path.
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    app.fake.timeout_after_success_initiate();
    let res = app.checkout(&buyer, METHOD).await;
    assert_failed_and_restored(&app, &l, &res).await;
}

#[sqlx::test]
async fn untrusted_payment_url_is_a_failure(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    app.fake
        .set_payment_url(Some("https://pay.fake.test.evil.example/x"));
    let res = app.checkout(&buyer, METHOD).await;
    assert_failed_and_restored(&app, &l, &res).await;
    assert!(!res.json.to_string().contains("evil"));
}

#[sqlx::test]
async fn expiring_twice_restores_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(app.available(l.variant_id).await, 3);
    let id: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();

    for expected in [true, false] {
        let mut tx = app.pool.begin().await.unwrap();
        let expired = server::checkout::expire_and_restore(&mut tx, id)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(expired, expected);
        assert_eq!(app.available(l.variant_id).await, 5);
    }
    assert_eq!(checkout_status(&app, &id.to_string()).await, "expired");

    // A paid checkout is never expired and its stock never comes back.
    app.set_cart(&buyer, l.variant_id, 1).await;
    let res = app.checkout(&buyer, METHOD).await;
    let id: Uuid = res.json["checkout_id"].as_str().unwrap().parse().unwrap();
    sqlx::query("UPDATE checkouts SET status = 'paid', paid_at = now() WHERE id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    assert!(
        !server::checkout::expire_and_restore(&mut tx, id)
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    assert_eq!(app.available(l.variant_id).await, 4);
}

#[sqlx::test]
async fn variant_in_order_cannot_be_deleted(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let product_id = app
        .add_product(
            &l.seller_token,
            l.store_id,
            100_000,
            &[(json!({"size": "M"}), 5), (json!({"size": "L"}), 5)],
        )
        .await;
    let medium: Uuid = sqlx::query_scalar(
        "SELECT id FROM product_variants WHERE product_id = $1 AND attributes = '{\"size\":\"M\"}'",
    )
    .bind(product_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, medium, 1).await;
    assert_eq!(app.checkout(&buyer, METHOD).await.status, 201);

    let path = format!("/products/{product_id}/variants");
    let res = app
        .request(
            Method::PUT,
            &path,
            Some(json!([{ "attributes": { "size": "L" }, "stock": 3 }])),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "variant_in_use");
    assert_eq!(app.variant_ids(product_id).await.len(), 2);

    // Keeping the ordered variant (and dropping one never ordered) is fine.
    let res = app
        .request(
            Method::PUT,
            &path,
            Some(json!([{ "attributes": { "size": "M" }, "stock": 3 }])),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(app.variant_ids(product_id).await, vec![medium]);
}

#[sqlx::test]
async fn checkout_total_cap(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1_000_000_000, 10).await;
    let cheap = app
        .add_product(&l.seller_token, l.store_id, 1, &[(json!({}), 10)])
        .await;
    let cheap = app.variant_ids(cheap).await[0];
    let (_, buyer) = app.sign_up_verified(BUYER).await;

    // Exactly the cap is allowed.
    app.set_cart(&buyer, l.variant_id, 5).await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["total_kobo"], MAX_CHECKOUT_TOTAL_KOBO);

    // Cap + 1 kobo is refused and reserves nothing.
    app.set_cart(&buyer, cheap, 1).await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "amount_too_large");
    assert_eq!(app.available(l.variant_id).await, 5);
    assert_eq!(app.available(cheap).await, 10);
}

#[sqlx::test]
async fn pending_checkout_cap(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 50).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    for _ in 0..3 {
        assert_eq!(app.checkout(&buyer, METHOD).await.status, 201);
    }
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 429, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "too_many_pending_checkouts");
    assert_eq!(app.available(l.variant_id).await, 47);

    // Once one is no longer pending, the buyer can check out again.
    sqlx::query(
        "UPDATE checkouts SET status = 'expired' \
         WHERE id = (SELECT id FROM checkouts ORDER BY created_at LIMIT 1)",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(app.checkout(&buyer, METHOD).await.status, 201);

    // The cap is per buyer.
    let (_, other) = app.sign_up_verified(BUYER2).await;
    app.set_cart(&other, l.variant_id, 1).await;
    assert_eq!(app.checkout(&other, METHOD).await.status, 201);
}

#[sqlx::test]
async fn idempotent_checkout_returns_same(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let body = app.checkout_body(&buyer, METHOD).await;
    let send = |key: &'static str, method: &'static str| {
        let buyer = buyer.clone();
        let app = &app;
        let mut body = body.clone();
        body["method"] = json!(method);
        async move {
            app.request_with_headers(
                Method::POST,
                "/checkout",
                Some(body),
                Some(&buyer),
                &[("idempotency-key", key)],
            )
            .await
        }
    };

    let first = send("order-1_abc", METHOD).await;
    assert_eq!(first.status, 201, "{}", first.json);
    let again = send("order-1_abc", METHOD).await;
    assert_eq!(again.status, 201, "{}", again.json);
    assert_eq!(again.json, first.json);
    assert_eq!(app.fake.calls("initiate"), 1);
    assert_eq!(count(&app, "SELECT count(*) FROM checkouts").await, 1);
    assert_eq!(app.available(l.variant_id).await, 4);

    // Same key, different request.
    let res = send("order-1_abc", "paystack_card").await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "idempotency_key_mismatch");

    // A different key is a new checkout (with a fresh delivery option: options are single-use).
    let res = send("order-2", METHOD).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "delivery_option_invalid");
    let fresh = app.checkout_body(&buyer, METHOD).await;
    let res = app
        .request_with_headers(
            Method::POST,
            "/checkout",
            Some(fresh),
            Some(&buyer),
            &[("idempotency-key", "order-2")],
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);

    // Bad keys are refused.
    let res = send("has space", METHOD).await;
    assert_eq!(res.status, 422, "{}", res.json);
    let long: &'static str = Box::leak("k".repeat(65).into_boxed_str());
    assert_eq!(send(long, METHOD).await.status, 422);

    // Keys are per buyer: another buyer's same key is their own checkout.
    let (_, other) = app.sign_up_verified(BUYER2).await;
    app.set_cart(&other, l.variant_id, 1).await;
    let other_body = app.checkout_body(&other, METHOD).await;
    let res = app
        .request_with_headers(
            Method::POST,
            "/checkout",
            Some(other_body),
            Some(&other),
            &[("idempotency-key", "order-1_abc")],
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_ne!(res.json["checkout_id"], first.json["checkout_id"]);
}

#[sqlx::test]
async fn idempotent_replay_of_failed_checkout_is_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    app.fake.fail_initiate();
    let body = app.checkout_body(&buyer, METHOD).await;
    let send = || {
        app.request_with_headers(
            Method::POST,
            "/checkout",
            Some(body.clone()),
            Some(&buyer),
            &[("idempotency-key", "k1")],
        )
    };
    assert_eq!(send().await.status, 502);
    let res = send().await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "checkout_not_pending");
    assert_eq!(app.fake.calls("initiate"), 1);
}

#[sqlx::test]
async fn checkout_uses_current_price(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    let preview = app.request(Method::GET, "/cart", None, Some(&buyer)).await;
    assert_eq!(preview.json["total_kobo"], 200_000);
    app.request(
        Method::PATCH,
        &format!("/products/{}", l.product_id),
        Some(json!({ "base_price_kobo": 120_000 })),
        Some(&l.seller_token),
    )
    .await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);
    assert_eq!(res.json["total_kobo"], 240_000);
}

#[sqlx::test]
async fn get_checkout_is_buyer_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    let created = app.checkout(&buyer, METHOD).await;
    let id = created.json["checkout_id"].as_str().unwrap();
    let path = format!("/checkout/{id}");

    let res = app.request(Method::GET, &path, None, Some(&buyer)).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["checkout_id"], id);
    assert_eq!(res.json["status"], "pending");
    assert_eq!(res.json["total_kobo"], 200_000);
    assert_eq!(res.json["payment_url"], created.json["payment_url"]);
    assert_eq!(res.json["orders"][0]["store_name"], "Mama Put");
    assert_eq!(res.json["orders"][0]["status"], "pending_payment");
    assert_eq!(res.json["orders"][0]["items"][0]["quantity"], 2);
    assert_eq!(
        res.json["orders"][0]["items"][0]["unit_price_kobo"],
        100_000
    );
    // The platform's cut is not the buyer's business.
    assert!(!res.json.to_string().contains("platform_cut"));

    let (_, other) = app.sign_up_verified(BUYER2).await;
    assert_eq!(
        app.request(Method::GET, &path, None, Some(&other))
            .await
            .status,
        404
    );
    assert_eq!(
        app.request(Method::GET, &path, None, Some(&l.seller_token))
            .await
            .status,
        404
    );
    assert_eq!(
        app.request(Method::GET, "/checkout/not-a-uuid", None, Some(&buyer))
            .await
            .status,
        404
    );
    assert_eq!(
        app.request(Method::GET, &path, None, None).await.status,
        401
    );
}

async fn put_stock(app: &TestApp, l: &Listing, stock: i32) -> common::TestResponse {
    app.request(
        Method::PUT,
        &format!("/products/{}/variants", l.product_id),
        Some(json!([{ "attributes": {}, "stock": stock }])),
        Some(&l.seller_token),
    )
    .await
}

async fn pending_checkout(app: &TestApp, l: &Listing, quantity: i64) -> Uuid {
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, quantity).await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);
    res.json["checkout_id"].as_str().unwrap().parse().unwrap()
}

#[sqlx::test]
async fn seller_edit_keeps_reservations(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let id = pending_checkout(&app, &l, 2).await;

    // The seller counts 6 on the shelf; the 2 held by the checkout stay held.
    let res = put_stock(&app, &l, 6).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["variants"][0]["stock"], 6);
    assert_eq!(res.json["variants"][0]["reserved_quantity"], 2);
    assert_eq!(res.json["variants"][0]["available_stock"], 4);

    let mut tx = app.pool.begin().await.unwrap();
    assert!(
        server::checkout::expire_and_restore(&mut tx, id)
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    // 6, not 8: expiry releases the reservation and never adds to on-hand stock.
    assert_eq!(app.available(l.variant_id).await, 6);
    assert_eq!(app.stock(l.variant_id).await, 6);
    assert_eq!(app.reserved(l.variant_id).await, 0);
}

#[sqlx::test]
async fn seller_cannot_set_stock_below_reserved(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    pending_checkout(&app, &l, 2).await;

    let res = put_stock(&app, &l, 1).await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "stock_below_reserved");
    assert_eq!(app.stock(l.variant_id).await, 5);
    assert_eq!(app.reserved(l.variant_id).await, 2);

    // Exactly the reserved amount is allowed: nothing left to buy.
    let res = put_stock(&app, &l, 2).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(app.available(l.variant_id).await, 0);

    // The database refuses it too.
    let direct = sqlx::query("UPDATE product_variants SET stock = 1 WHERE id = $1")
        .bind(l.variant_id)
        .execute(&app.pool)
        .await;
    assert!(direct.is_err());
}

#[sqlx::test]
async fn commit_reserved_moves_units(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let id = pending_checkout(&app, &l, 2).await;
    assert_eq!(
        (
            app.stock(l.variant_id).await,
            app.reserved(l.variant_id).await
        ),
        (5, 2)
    );

    let mut tx = app.pool.begin().await.unwrap();
    server::checkout::commit_reserved(&mut tx, id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(app.stock(l.variant_id).await, 3);
    assert_eq!(app.reserved(l.variant_id).await, 0);
    assert_eq!(app.available(l.variant_id).await, 3);
}

#[sqlx::test]
async fn concurrent_same_idempotency_key_creates_one(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    let body = app.checkout_body(&buyer, METHOD).await;
    let send = || {
        app.request_with_headers(
            Method::POST,
            "/checkout",
            Some(body.clone()),
            Some(&buyer),
            &[("idempotency-key", "same-key")],
        )
    };
    let (r1, r2) = tokio::join!(send(), send());
    let mut statuses = [r1.status, r2.status];
    statuses.sort();
    match statuses {
        [201, 201] => assert_eq!(r1.json, r2.json),
        [201, 409] => {
            let loser = if r1.status == 409 { &r1 } else { &r2 };
            assert_eq!(loser.json["error"]["code"], "checkout_in_progress");
        }
        other => panic!("unexpected {other:?}: {} / {}", r1.json, r2.json),
    }
    assert_eq!(count(&app, "SELECT count(*) FROM checkouts").await, 1);
    assert_eq!(app.reserved(l.variant_id).await, 2);
    assert_eq!(app.fake.calls("initiate"), 1);
}

#[sqlx::test]
async fn replay_of_paid_checkout_is_not_pending(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let body = app.checkout_body(&buyer, METHOD).await;
    let send = || {
        app.request_with_headers(
            Method::POST,
            "/checkout",
            Some(body.clone()),
            Some(&buyer),
            &[("idempotency-key", "k1")],
        )
    };
    assert_eq!(send().await.status, 201);
    sqlx::query("UPDATE checkouts SET status = 'paid', paid_at = now()")
        .execute(&app.pool)
        .await
        .unwrap();
    let res = send().await;
    assert_eq!(res.status, 409, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "checkout_not_pending");
    assert!(res.json.get("payment_url").is_none());
}

#[sqlx::test]
async fn failure_to_store_payment_page_expires_checkout(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 2).await;
    // Make saving the payment page fail after a successful initiate.
    sqlx::raw_sql(
        "CREATE FUNCTION fail_payment_url() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'injected failure'; END $$; \
         CREATE TRIGGER fail_payment_url BEFORE UPDATE OF payment_url ON checkouts \
         FOR EACH ROW WHEN (NEW.payment_url IS NOT NULL) EXECUTE FUNCTION fail_payment_url();",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(app.fake.calls("initiate"), 1);
    assert_failed_and_restored(&app, &l, &res).await;
}

// ---- Delivery at checkout (Part 3, Task 6) ----

type Quotes = Vec<(Uuid, Vec<serde_json::Value>)>;

/// `(store_id, options)` for every store in the buyer's delivery-options answer.
async fn quoted(app: &TestApp, buyer: &str, address: Uuid) -> Quotes {
    let res = app.delivery_options(buyer, address).await;
    assert_eq!(res.status, 200, "{}", res.json);
    res.json["stores"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["store_id"].as_str().unwrap().parse().unwrap(),
                s["options"].as_array().unwrap().clone(),
            )
        })
        .collect()
}

/// The id of the first option with `method` quoted for `store`.
fn option_id(quotes: &Quotes, store: Uuid, method: &str) -> Uuid {
    let (_, options) = quotes.iter().find(|(s, _)| *s == store).unwrap();
    options
        .iter()
        .find(|o| o["method"] == method)
        .unwrap_or_else(|| panic!("no {method} option: {options:?}"))["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

/// The checkout transaction holds locks while it checks the chosen options, so
/// it must read on its own connection: with one connection in the pool, a
/// second acquire inside it can only time out.
#[sqlx::test]
async fn checkout_runs_on_a_pool_of_one(pool: PgPool) {
    let one = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect_with((*pool.connect_options()).clone())
        .await
        .unwrap();
    let app = TestApp::new(one).await;
    let a = listing(&app, 100_000, 5).await;
    let b = app
        .make_active_store_with_product("08011111112", "Bukka Hut", 50_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, a.variant_id, 1).await;
    app.set_cart(&buyer, b.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let quotes = quoted(&app, &buyer, address).await;
    let courier = option_id(&quotes, a.store_id, "courier");
    let own = option_id(&quotes, b.store_id, "self");

    let res = app
        .checkout_with(
            &buyer,
            METHOD,
            address,
            &[(a.store_id, courier), (b.store_id, own)],
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    app.pool.close().await;
}

/// Retention (plan 8) must be able to clear the buyer's address snapshot, but
/// nothing else may leave a delivery order without one.
#[sqlx::test]
async fn address_snapshot_clears_only_with_a_purge_stamp(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 10_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);

    let err = sqlx::query("UPDATE orders SET delivery_address_enc = NULL")
        .execute(&app.pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().unwrap().constraint(),
        Some("orders_delivery_details")
    );
    sqlx::query(
        "UPDATE orders SET delivery_address_enc = NULL, delivery_address_purged_at = now()",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM orders WHERE delivery_address_enc IS NULL"
        )
        .await,
        1
    );
}

fn error_code(res: &common::TestResponse) -> &str {
    res.json["error"]["code"].as_str().unwrap_or("")
}

#[sqlx::test]
async fn checkout_charges_items_plus_delivery(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let a = app
        .make_active_store_with_product("08011111111", "Mama Put", 100_000, 5)
        .await;
    let b = app
        .make_active_store_with_product("08011111112", "Bukka Hut", 50_000, 5)
        .await;
    let res = app
        .request(
            Method::PATCH,
            &format!("/stores/{}", b.store_id),
            Some(json!({ "self_delivery_fee_kobo": 70_000 })),
            Some(&b.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);
    app.courier.set_rates(vec![server::couriers::Rate {
        courier_id: "kwik".into(),
        service_code: "kwik-std".into(),
        courier_name: "Kwik".into(),
        amount_kobo: 140_000,
        charged_kobo: 150_000,
        delivery_eta: Some("Same day".into()),
    }]);
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, a.variant_id, 2).await;
    app.set_cart(&buyer, b.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let quotes = quoted(&app, &buyer, address).await;
    let courier = option_id(&quotes, a.store_id, "courier");
    let own = option_id(&quotes, b.store_id, "self");

    let res = app
        .checkout_with(
            &buyer,
            METHOD,
            address,
            &[(a.store_id, courier), (b.store_id, own)],
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    // Items 200_000 + 50_000, delivery 150_000 (courier charge) + 70_000 (self fee).
    assert_eq!(res.json["total_kobo"], 470_000);
    let sent = app.fake.initiated();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].amount_kobo, 470_000);
    let mut fees: Vec<(String, i64)> = res.json["orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            (
                o["store_name"].as_str().unwrap().to_string(),
                o["delivery_fee_kobo"].as_i64().unwrap(),
            )
        })
        .collect();
    fees.sort();
    assert_eq!(
        fees,
        vec![("Bukka Hut".into(), 70_000), ("Mama Put".into(), 150_000)]
    );

    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        Uuid,
        i64,
        i64,
        Option<String>,
        Option<Uuid>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        bool,
    )> = sqlx::query_as(
        "SELECT store_id, items_total_kobo, delivery_fee_kobo, delivery_method::text, \
                delivery_option_id, delivery_area::text, courier_request_token, courier_id, \
                service_code, courier_quote_kobo, promised_date IS NOT NULL \
         FROM orders ORDER BY items_total_kobo DESC",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                a.store_id,
                200_000,
                150_000,
                Some("courier".into()),
                Some(courier),
                Some("amac".into()),
                Some("fake-token".into()),
                Some("kwik".into()),
                Some("kwik-std".into()),
                Some(150_000),
                true
            ),
            (
                b.store_id,
                50_000,
                70_000,
                Some("self".into()),
                Some(own),
                Some("amac".into()),
                None,
                None,
                None,
                None,
                true
            ),
        ]
    );
    let sum: i64 = rows.iter().map(|r| r.1 + r.2).sum();
    assert_eq!(sum, 470_000);
    // The courier order keeps the sender address code its quote was made from.
    let (order_sender, store_code): (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT o.courier_sender_code, s.pickup_courier_address_code \
         FROM orders o JOIN stores s ON s.id = o.store_id WHERE o.store_id = $1",
    )
    .bind(a.store_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(order_sender.is_some());
    assert_eq!(order_sender, store_code);
    let self_sender: Option<i64> =
        sqlx::query_scalar("SELECT courier_sender_code FROM orders WHERE store_id = $1")
            .bind(b.store_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(self_sender, None);
    let checkout_id = res.json["checkout_id"].as_str().unwrap();
    let view = app
        .request(
            Method::GET,
            &format!("/checkout/{checkout_id}"),
            None,
            Some(&buyer),
        )
        .await;
    assert_eq!(view.json["total_kobo"], 470_000);
}

#[sqlx::test]
async fn checkout_refuses_expired_or_foreign_option(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    let (_, other) = app.sign_up_verified(BUYER2).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    app.set_cart(&other, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let other_address = app.buyer_address(&other).await;
    let mine = option_id(&quoted(&app, &buyer, address).await, l.store_id, "self");
    let theirs = option_id(
        &quoted(&app, &other, other_address).await,
        l.store_id,
        "self",
    );
    let refused = |res: common::TestResponse, what: &str| {
        assert_eq!(res.status, 422, "{what}: {}", res.json);
        assert_eq!(error_code(&res), "delivery_option_invalid", "{what}");
    };

    refused(
        app.checkout_with(&buyer, METHOD, address, &[(l.store_id, Uuid::new_v4())])
            .await,
        "unknown",
    );
    refused(
        app.checkout_with(&buyer, METHOD, address, &[(l.store_id, theirs)])
            .await,
        "another buyer's",
    );
    // Quoted for one address, shipped to another (ruling P3).
    let second_address = app.add_address(&buyer, "bwari").await;
    refused(
        app.checkout_with(&buyer, METHOD, second_address, &[(l.store_id, mine)])
            .await,
        "another address",
    );
    // The cart changed after quoting (ruling T6a); restoring it makes the option valid again.
    app.set_cart(&buyer, l.variant_id, 2).await;
    refused(
        app.checkout_with(&buyer, METHOD, address, &[(l.store_id, mine)])
            .await,
        "cart changed",
    );
    app.set_cart(&buyer, l.variant_id, 1).await;
    sqlx::query(
        "UPDATE delivery_options SET expires_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(mine)
    .execute(&app.pool)
    .await
    .unwrap();
    refused(
        app.checkout_with(&buyer, METHOD, address, &[(l.store_id, mine)])
            .await,
        "expired",
    );
    assert_eq!(count(&app, "SELECT count(*) FROM checkouts").await, 0);
    assert_eq!(app.reserved(l.variant_id).await, 0);

    let fresh = option_id(&quoted(&app, &buyer, address).await, l.store_id, "self");
    let res = app
        .checkout_with(&buyer, METHOD, address, &[(l.store_id, fresh)])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
}

#[sqlx::test]
async fn checkout_requires_one_option_per_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let a = app
        .make_active_store_with_product("08011111111", "Mama Put", 100_000, 5)
        .await;
    let b = app
        .make_active_store_with_product("08011111112", "Bukka Hut", 50_000, 5)
        .await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, a.variant_id, 1).await;
    app.set_cart(&buyer, b.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let quotes = quoted(&app, &buyer, address).await;
    let oa = option_id(&quotes, a.store_id, "self");
    let ob = option_id(&quotes, b.store_id, "self");

    let res = app
        .checkout_with(&buyer, METHOD, address, &[(a.store_id, oa)])
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "delivery_option_required");
    let res = app.checkout_with(&buyer, METHOD, address, &[]).await;
    assert_eq!(error_code(&res), "delivery_option_required");
    // Swapped: each option belongs to the other store.
    let res = app
        .checkout_with(
            &buyer,
            METHOD,
            address,
            &[(a.store_id, ob), (b.store_id, oa)],
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "delivery_option_invalid");
    // Two choices for one store.
    let res = app
        .checkout_with(
            &buyer,
            METHOD,
            address,
            &[(a.store_id, oa), (a.store_id, oa), (b.store_id, ob)],
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "delivery_option_invalid");
    // A store that is not in the cart.
    let res = app
        .checkout_with(
            &buyer,
            METHOD,
            address,
            &[(a.store_id, oa), (b.store_id, ob), (Uuid::new_v4(), oa)],
        )
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "delivery_option_invalid");
    assert_eq!(count(&app, "SELECT count(*) FROM checkouts").await, 0);

    let res = app
        .checkout_with(
            &buyer,
            METHOD,
            address,
            &[(a.store_id, oa), (b.store_id, ob)],
        )
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
}

#[sqlx::test]
async fn checkout_refuses_address_outside_fct(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let option = option_id(&quoted(&app, &buyer, address).await, l.store_id, "self");
    // Forced past the API: an area the service does not cover.
    sqlx::query("ALTER TYPE area_council ADD VALUE 'suleja'")
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE buyer_addresses SET area_council = 'suleja'")
        .execute(&app.pool)
        .await
        .unwrap();

    let res = app
        .checkout_with(&buyer, METHOD, address, &[(l.store_id, option)])
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "outside_service_area");
    assert_eq!(res.json["error"]["message"], "Coming to your city soon");
    let res = app.delivery_options(&buyer, address).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "outside_service_area");
    assert_eq!(count(&app, "SELECT count(*) FROM checkouts").await, 0);
    assert_eq!(app.reserved(l.variant_id).await, 0);

    // Someone else's address is not found.
    let (_, other) = app.sign_up_verified(BUYER2).await;
    app.set_cart(&other, l.variant_id, 1).await;
    let res = app
        .checkout_with(&other, METHOD, address, &[(l.store_id, option)])
        .await;
    assert_eq!(res.status, 404, "{}", res.json);
}

/// Waits out 23:59 Africa/Lagos, the one minute in which a 23:59 cut-off is not
/// "before the cut-off".
async fn avoid_last_lagos_minute() {
    let lagos =
        time::OffsetDateTime::now_utc().to_offset(time::UtcOffset::from_hms(1, 0, 0).unwrap());
    if lagos.hour() == 23 && lagos.minute() == 59 {
        let wait = 61 - u64::from(lagos.second());
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
    }
}

#[sqlx::test]
async fn promised_date_snapshotted_at_payment(pool: PgPool) {
    use server::{
        checkout::{SettleOutcome, settle_payment},
        delivery::promise::{load_holidays, promised_date},
        payments::PaymentStatus,
    };
    avoid_last_lagos_minute().await;
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    // Quoted before the cut-off (23:59)...
    sqlx::query("UPDATE platform_settings SET delivery_cutoff_minutes = 1439")
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let quotes = quoted(&app, &buyer, address).await;
    let shown = quotes[0].1.iter().find(|o| o["method"] == "self").unwrap()["promised_date"]
        .as_str()
        .unwrap()
        .to_string();
    let option = option_id(&quotes, l.store_id, "self");
    let res = app
        .checkout_with(&buyer, METHOD, address, &[(l.store_id, option)])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    let at_checkout: time::Date = sqlx::query_scalar("SELECT promised_date FROM orders")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(at_checkout.to_string(), shown);

    // ...and paid after it (a cut-off of 00:00 means every time is after).
    sqlx::query("UPDATE platform_settings SET delivery_cutoff_minutes = 0")
        .execute(&app.pool)
        .await
        .unwrap();
    let reference = res.json["reference"].as_str().unwrap().to_string();
    app.fake.set_payment(
        &reference,
        PaymentStatus::Successful {
            amount_kobo: 100_000,
            fee_kobo: 0,
            currency: "NGN".into(),
            held: false,
            reference: reference.clone(),
        },
    );
    assert_eq!(
        settle_payment(&app.state, &reference).await.unwrap(),
        SettleOutcome::Paid
    );
    let (paid_at, promised): (time::OffsetDateTime, time::Date) =
        sqlx::query_as("SELECT paid_at, promised_date FROM orders")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let holidays = load_holidays(&app.pool).await.unwrap();
    assert_eq!(promised, promised_date(paid_at, 0, &holidays));
    assert!(
        promised > at_checkout,
        "paid after the cut-off moves the date later: {promised} vs shown {shown}"
    );
}

#[sqlx::test]
async fn order_address_snapshot_encrypted(pool: PgPool) {
    use server::delivery::addresses::DeliveryAddress;
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let res = app.checkout(&buyer, METHOD).await;
    assert_eq!(res.status, 201, "{}", res.json);
    let address = app.buyer_address(&buyer).await;

    let (blob, area): (Vec<u8>, String) =
        sqlx::query_as("SELECT delivery_address_enc, delivery_area::text FROM orders")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(area, "amac");
    let raw = String::from_utf8_lossy(&blob);
    for secret in [
        common::BUYER_ADDRESS_LINE,
        "Ada Buyer",
        "7777777",
        "blue gate",
    ] {
        assert!(!raw.contains(secret), "plaintext {secret} in the snapshot");
    }
    let snapshot: DeliveryAddress =
        serde_json::from_str(&app.state.cipher.decrypt(&blob).unwrap()).unwrap();
    assert_eq!(snapshot.id, address);
    assert_eq!(snapshot.address_line, common::BUYER_ADDRESS_LINE);
    assert_eq!(snapshot.recipient_name, "Ada Buyer");
    assert_eq!(snapshot.phone, "+2348077777777");
    assert_eq!(snapshot.landmark.as_deref(), Some("Opposite the blue gate"));

    // The checkout view never echoes the address.
    let id = res.json["checkout_id"].as_str().unwrap();
    let view = app
        .request(Method::GET, &format!("/checkout/{id}"), None, Some(&buyer))
        .await;
    assert!(
        !view.json.to_string().contains("Quiet Close"),
        "{}",
        view.json
    );
}

// ---- Fix round 1: single-use options, method re-check ----

/// Pays a checkout through the fake provider (amount `total`, no fee).
async fn pay(app: &TestApp, reference: &str, total: i64) {
    use server::{
        checkout::{SettleOutcome, settle_payment},
        payments::PaymentStatus,
    };
    app.fake.set_payment(
        reference,
        PaymentStatus::Successful {
            amount_kobo: total,
            fee_kobo: 0,
            currency: "NGN".into(),
            held: false,
            reference: reference.to_string(),
        },
    );
    assert_eq!(
        settle_payment(&app.state, reference).await.unwrap(),
        SettleOutcome::Paid
    );
}

#[sqlx::test]
async fn option_is_single_use_across_checkouts(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let option = option_id(&quoted(&app, &buyer, address).await, l.store_id, "self");
    let chosen = [(l.store_id, option)];
    let choose = || app.checkout_with(&buyer, METHOD, address, &chosen);

    let first = choose().await;
    assert_eq!(first.status, 201, "{}", first.json);
    // A second pending checkout cannot reuse it.
    let res = choose().await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "delivery_option_invalid");

    // Once that checkout has expired (orders cancelled) the option is free again.
    let first_id: Uuid = first.json["checkout_id"].as_str().unwrap().parse().unwrap();
    let mut tx = app.pool.begin().await.unwrap();
    assert!(
        server::checkout::expire_and_restore(&mut tx, first_id)
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    let second = choose().await;
    assert_eq!(second.status, 201, "{}", second.json);

    // Paid, the cart cleared, the same lines added again: still refused.
    pay(&app, second.json["reference"].as_str().unwrap(), 100_000).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let res = choose().await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "delivery_option_invalid");
}

#[sqlx::test]
async fn courier_quote_is_used_once(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let rate = |id: &str, kobo: i64| server::couriers::Rate {
        courier_id: id.into(),
        service_code: format!("{id}-std"),
        courier_name: id.into(),
        amount_kobo: kobo,
        charged_kobo: kobo,
        delivery_eta: None,
    };
    app.courier
        .set_rates(vec![rate("kwik", 150_000), rate("gig", 160_000)]);
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let quotes = quoted(&app, &buyer, address).await;
    let couriers: Vec<Uuid> = quotes[0]
        .1
        .iter()
        .filter(|o| o["method"] == "courier")
        .map(|o| o["id"].as_str().unwrap().parse().unwrap())
        .collect();
    assert_eq!(couriers.len(), 2);
    let res = app
        .checkout_with(&buyer, METHOD, address, &[(l.store_id, couriers[0])])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
    // The other rate of the same quote shares its request token: refused.
    let res = app
        .checkout_with(&buyer, METHOD, address, &[(l.store_id, couriers[1])])
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "delivery_option_invalid");
}

#[sqlx::test]
async fn checkout_refuses_a_method_the_store_no_longer_offers(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 100_000, 5).await;
    let (_, buyer) = app.sign_up_verified(BUYER).await;
    app.set_cart(&buyer, l.variant_id, 1).await;
    let address = app.buyer_address(&buyer).await;
    let quotes = quoted(&app, &buyer, address).await;
    let own = option_id(&quotes, l.store_id, "self");
    let courier = option_id(&quotes, l.store_id, "courier");
    let store_path = format!("/stores/{}", l.store_id);
    let patch = |body: serde_json::Value| {
        app.request(
            Method::PATCH,
            &store_path,
            Some(body),
            Some(&l.seller_token),
        )
    };

    assert_eq!(patch(json!({ "delivery_self": false })).await.status, 200);
    let res = app
        .checkout_with(&buyer, METHOD, address, &[(l.store_id, own)])
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "delivery_option_invalid");

    // The pickup moved: the courier quote's sender address is stale.
    assert_eq!(
        patch(json!({ "pickup_area_council": "kuje" })).await.status,
        200
    );
    let res = app
        .checkout_with(&buyer, METHOD, address, &[(l.store_id, courier)])
        .await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(error_code(&res), "delivery_option_invalid");
    assert_eq!(count(&app, "SELECT count(*) FROM checkouts").await, 0);

    // A fresh quote from the new pickup works.
    let fresh = option_id(&quoted(&app, &buyer, address).await, l.store_id, "courier");
    let res = app
        .checkout_with(&buyer, METHOD, address, &[(l.store_id, fresh)])
        .await;
    assert_eq!(res.status, 201, "{}", res.json);
}
