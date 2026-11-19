//! The product feed, search and categories (plan 7, task S1).

mod common;

use axum::http::Method;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn get(app: &TestApp, path: &str) -> TestResponse {
    app.request(Method::GET, path, None, None).await
}

fn names(v: &Value) -> Vec<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect()
}

fn ids(v: &Value) -> Vec<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

async fn set(
    pool: &PgPool,
    product: Uuid,
    name: &str,
    brand: Option<&str>,
    category: &str,
    condition: &str,
) {
    sqlx::query(
        "UPDATE products SET name = $2, brand = $3, category = $4, condition = $5::product_condition \
         WHERE id = $1",
    )
    .bind(product)
    .bind(name)
    .bind(brand)
    .bind(category)
    .bind(condition)
    .execute(pool)
    .await
    .unwrap();
}

/// A store with one published product, renamed and re-priced.
async fn product(
    app: &TestApp,
    phone: &str,
    store: &str,
    name: &str,
    price: i64,
) -> common::Listing {
    let l = app
        .make_active_store_with_product(phone, store, price, 5)
        .await;
    sqlx::query("UPDATE products SET name = $2 WHERE id = $1")
        .bind(l.product_id)
        .bind(name)
        .execute(&app.pool)
        .await
        .unwrap();
    l
}

#[sqlx::test]
async fn only_published_products_of_active_stores_appear_and_no_sign_in_is_needed(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let a = product(&app, "08031110001", "Alpha", "Visible bag", 500_000).await;
    let b = product(&app, "08031110002", "Beta", "Suspended shop item", 500_000).await;
    let c = product(&app, "08031110003", "Gamma", "Draft item", 500_000).await;
    let d = product(&app, "08031110004", "Delta", "Deleted item", 500_000).await;
    sqlx::query("UPDATE stores SET status = 'suspended', suspended_at = now() WHERE id = $1")
        .bind(b.store_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET is_published = false WHERE id = $1")
        .bind(c.product_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET deleted_at = now() WHERE id = $1")
        .bind(d.product_id)
        .execute(&pool)
        .await
        .unwrap();

    let res = get(&app, "/products").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(names(&res.json), vec!["Visible bag"]);
    assert_eq!(res.json["items"][0]["id"], a.product_id.to_string());
    assert!(res.json["next_cursor"].is_null());
}

#[sqlx::test]
async fn a_card_carries_what_the_app_needs_and_no_seller_details(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let l = product(&app, "08031110001", "Alpha Threads", "Ankara bag", 750_000).await;
    set(
        &pool,
        l.product_id,
        "Ankara bag",
        Some("Zuri"),
        "fashion",
        "used",
    )
    .await;
    sqlx::query("UPDATE stores SET rating_count = 2, rating_sum = 9 WHERE id = $1")
        .bind(l.store_id)
        .execute(&pool)
        .await
        .unwrap();

    let item = get(&app, "/products").await.json["items"][0].clone();
    assert_eq!(item["name"], "Ankara bag");
    assert_eq!(item["base_price_kobo"], 750_000);
    assert_eq!(item["cover_photo_key"], "photos/a.jpg");
    assert_eq!(item["brand"], "Zuri");
    assert_eq!(item["condition"], "used");
    assert_eq!(item["store_name"], "Alpha Threads");
    assert_eq!(item["store_id"], l.store_id.to_string());
    assert!(item["store_slug"].is_string());
    assert_eq!(item["rating_count"], 2);
    assert_eq!(item["rating_average"], 4.5);
    assert_eq!(item["in_stock"], true);
    let text = item.to_string();
    for forbidden in ["owner", "phone", "address", "pickup"] {
        assert!(!text.contains(forbidden), "{forbidden} in {text}");
    }
}

#[sqlx::test]
async fn a_store_with_no_reviews_has_no_average(pool: PgPool) {
    let app = TestApp::new(pool).await;
    product(&app, "08031110001", "Alpha", "Bag", 500_000).await;
    let item = get(&app, "/products").await.json["items"][0].clone();
    assert_eq!(item["rating_count"], 0);
    assert!(item["rating_average"].is_null());
}

#[sqlx::test]
async fn out_of_stock_items_are_flagged_not_hidden(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let a = app
        .make_active_store_with_product("08031110001", "Alpha", 500_000, 2)
        .await;
    let b = app
        .make_active_store_with_product("08031110002", "Beta", 500_000, 0)
        .await;
    sqlx::query("UPDATE products SET name = 'Has stock' WHERE id = $1")
        .bind(a.product_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE products SET name = 'Sold out' WHERE id = $1")
        .bind(b.product_id)
        .execute(&pool)
        .await
        .unwrap();
    let res = get(&app, "/products?sort=price_asc").await;
    let by_name = |n: &str| {
        res.json["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == n)
            .unwrap()
            .clone()
    };
    assert_eq!(by_name("Has stock")["in_stock"], true);
    assert_eq!(by_name("Sold out")["in_stock"], false);

    // Every unit held by a pending checkout counts as gone.
    sqlx::query("UPDATE product_variants SET reserved_quantity = 2 WHERE product_id = $1")
        .bind(a.product_id)
        .execute(&pool)
        .await
        .unwrap();
    let res = get(&app, "/products").await;
    assert!(
        res.json["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["in_stock"] == false)
    );
}

#[sqlx::test]
async fn search_finds_by_name_or_brand_ignoring_case_and_treats_wildcards_literally(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let a = product(&app, "08031110001", "Alpha", "Leather Handbag", 500_000).await;
    let b = product(&app, "08031110002", "Beta", "Phone case", 500_000).await;
    let c = product(&app, "08031110003", "Gamma", "100% cotton shirt", 500_000).await;
    set(
        &pool,
        a.product_id,
        "Leather Handbag",
        Some("Zuri"),
        "fashion",
        "new",
    )
    .await;
    set(
        &pool,
        b.product_id,
        "Phone case",
        Some("Tecno"),
        "phones",
        "new",
    )
    .await;
    set(
        &pool,
        c.product_id,
        "100% cotton shirt",
        None,
        "fashion",
        "new",
    )
    .await;

    assert_eq!(
        names(&get(&app, "/products?q=handbag").await.json),
        vec!["Leather Handbag"]
    );
    assert_eq!(
        names(&get(&app, "/products?q=LEATHER").await.json),
        vec!["Leather Handbag"]
    );
    assert_eq!(
        names(&get(&app, "/products?q=zur").await.json),
        vec!["Leather Handbag"]
    );
    assert_eq!(
        names(&get(&app, "/products?q=tecno").await.json),
        vec!["Phone case"]
    );
    assert!(names(&get(&app, "/products?q=nothing-like-this").await.json).is_empty());
    // A literal percent sign, not "match everything".
    assert_eq!(
        names(&get(&app, "/products?q=100%25").await.json),
        vec!["100% cotton shirt"]
    );
    assert!(names(&get(&app, "/products?q=%25zzz").await.json).is_empty());
    // Blank means no search; too long is refused.
    assert_eq!(names(&get(&app, "/products?q=%20%20").await.json).len(), 3);
    let long = "a".repeat(81);
    assert_eq!(get(&app, &format!("/products?q={long}")).await.status, 422);
}

#[sqlx::test]
async fn filters_by_category_condition_and_price(pool: PgPool) {
    let app = TestApp::new(pool.clone()).await;
    let a = product(&app, "08031110001", "Alpha", "Cheap new shirt", 100_000).await;
    let b = product(&app, "08031110002", "Beta", "Mid used phone", 500_000).await;
    let c = product(
        &app,
        "08031110003",
        "Gamma",
        "Dear refurbished laptop",
        2_000_000,
    )
    .await;
    set(
        &pool,
        a.product_id,
        "Cheap new shirt",
        None,
        "Fashion",
        "new",
    )
    .await;
    set(
        &pool,
        b.product_id,
        "Mid used phone",
        None,
        "phones",
        "used",
    )
    .await;
    set(
        &pool,
        c.product_id,
        "Dear refurbished laptop",
        None,
        "electronics",
        "refurbished",
    )
    .await;

    assert_eq!(
        names(&get(&app, "/products?category=fashion").await.json),
        vec!["Cheap new shirt"]
    );
    assert_eq!(
        names(&get(&app, "/products?condition=used").await.json),
        vec!["Mid used phone"]
    );
    let mut mid = names(
        &get(
            &app,
            "/products?min_price_kobo=200000&max_price_kobo=1000000",
        )
        .await
        .json,
    );
    mid.sort();
    assert_eq!(mid, vec!["Mid used phone"]);
    assert_eq!(
        names(&get(&app, "/products?min_price_kobo=1000000").await.json),
        vec!["Dear refurbished laptop"]
    );
    assert_eq!(
        names(&get(&app, "/products?max_price_kobo=100000").await.json),
        vec!["Cheap new shirt"]
    );
    // Filters combine.
    assert!(
        names(
            &get(&app, "/products?category=phones&condition=new")
                .await
                .json
        )
        .is_empty()
    );
}

#[sqlx::test]
async fn sorts_newest_first_by_default_and_by_price(pool: PgPool) {
    let app = TestApp::new(pool).await;
    product(&app, "08031110001", "Alpha", "First", 300_000).await;
    product(&app, "08031110002", "Beta", "Second", 100_000).await;
    product(&app, "08031110003", "Gamma", "Third", 200_000).await;

    assert_eq!(
        names(&get(&app, "/products").await.json),
        vec!["Third", "Second", "First"]
    );
    assert_eq!(
        names(&get(&app, "/products?sort=newest").await.json),
        vec!["Third", "Second", "First"]
    );
    assert_eq!(
        names(&get(&app, "/products?sort=price_asc").await.json),
        vec!["Second", "Third", "First"]
    );
    assert_eq!(
        names(&get(&app, "/products?sort=price_desc").await.json),
        vec!["First", "Third", "Second"]
    );
}

async fn walk(app: &TestApp, base: &str, limit: usize) -> Vec<String> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let url = match &cursor {
            Some(c) => format!("{base}&limit={limit}&before={c}"),
            None => format!("{base}&limit={limit}"),
        };
        let res = get(app, &url).await;
        assert_eq!(res.status, 200, "{url}: {}", res.json);
        all.extend(ids(&res.json));
        match res.json["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => return all,
        }
    }
    panic!("never reached the last page");
}

#[sqlx::test]
async fn every_sort_pages_through_everything_once_even_with_equal_prices(pool: PgPool) {
    let app = TestApp::new(pool).await;
    // One seller, many products at only two different prices, so ties must be broken by id.
    let l = app
        .make_active_store_with_product("08031110001", "Alpha", 100_000, 5)
        .await;
    for i in 0..6 {
        let price = if i % 2 == 0 { 100_000 } else { 200_000 };
        app.add_product(&l.seller_token, l.store_id, price, &[(json!({}), 3)])
            .await;
    }
    for sort in ["newest", "price_asc", "price_desc"] {
        let all = walk(&app, &format!("/products?sort={sort}"), 3).await;
        let mut unique = all.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(all.len(), 7, "{sort}: {all:?}");
        assert_eq!(unique.len(), 7, "{sort} repeats a product");
    }
    // The default page size covers a small list in one go.
    assert!(get(&app, "/products").await.json["next_cursor"].is_null());
}

#[sqlx::test]
async fn bad_input_is_refused_with_a_clear_422(pool: PgPool) {
    let app = TestApp::new(pool).await;
    product(&app, "08031110001", "Alpha", "Bag", 500_000).await;
    for bad in [
        "before=nope",
        "sort=cheapest",
        "condition=broken",
        "limit=0",
        "limit=101",
        "min_price_kobo=-1",
        "min_price_kobo=500&max_price_kobo=100",
        "min_price_kobo=abc",
    ] {
        let res = get(&app, &format!("/products?{bad}")).await;
        assert_eq!(res.status, 422, "{bad}: {}", res.json);
    }
    // A cursor from one sort is not valid in another.
    let first = get(&app, "/products?sort=price_asc&limit=1").await;
    assert!(first.json["next_cursor"].is_null());
}

#[sqlx::test]
async fn a_cursor_from_another_sort_is_refused(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product("08031110001", "Alpha", 100_000, 5)
        .await;
    app.add_product(&l.seller_token, l.store_id, 200_000, &[(json!({}), 3)])
        .await;
    let page = get(&app, "/products?sort=newest&limit=1").await;
    let cursor = page.json["next_cursor"].as_str().unwrap().to_string();
    let res = get(&app, &format!("/products?sort=price_asc&before={cursor}")).await;
    assert_eq!(res.status, 422, "{}", res.json);
}

#[sqlx::test]
async fn categories_are_a_fixed_list_anyone_can_read(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = get(&app, "/categories").await;
    assert_eq!(res.status, 200, "{}", res.json);
    let items = res.json["items"].as_array().unwrap();
    assert!(items.len() >= 8);
    let slugs: Vec<&str> = items.iter().map(|c| c["slug"].as_str().unwrap()).collect();
    assert!(
        slugs.contains(&"fashion") && slugs.contains(&"phones-tablets") && slugs.contains(&"other")
    );
    assert_eq!(items[0]["name"], "Fashion");
    // The last one is the catch-all.
    assert_eq!(items.last().unwrap()["slug"], "other");
    let mut seen = slugs.clone();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), slugs.len(), "duplicate slugs");
}
