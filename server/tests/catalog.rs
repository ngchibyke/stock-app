//! The public catalog: categories, browse and search, the store directory
//! (plan 7a, tasks 1 to 3).

mod common;

use axum::http::Method;
use common::TestApp;
use serde_json::{Value, json};
use sqlx::PgPool;

fn product_body(category: &str) -> Value {
    json!({
        "name": "Ankara Shirt",
        "description": "Handmade shirt",
        "category": category,
        "condition": "new",
        "photo_keys": ["photos/a.jpg"],
        "weight_grams": 300,
        "base_price_kobo": 1_200_000,
        "variants": [{ "attributes": { "size": "M" }, "stock": 4 }]
    })
}

#[sqlx::test]
async fn categories_are_listed_in_order_without_auth(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let res = app.request(Method::GET, "/categories", None, None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let items = res.json.as_array().unwrap();
    assert_eq!(items.len(), 10);
    assert_eq!(items[0], json!({ "slug": "fashion", "name": "Fashion" }));
    assert_eq!(items[9], json!({ "slug": "other", "name": "Other" }));
    let slugs: Vec<&str> = items.iter().map(|i| i["slug"].as_str().unwrap()).collect();
    assert_eq!(
        slugs,
        [
            "fashion",
            "phones-gadgets",
            "electronics",
            "home-kitchen",
            "beauty-personal-care",
            "baby-kids",
            "sports-fitness",
            "books-stationery",
            "food-groceries",
            "other"
        ]
    );
}

#[sqlx::test]
async fn product_needs_a_known_category(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = app
        .make_active_store_with_product("08055555551", "Kano Fabrics", 10_000, 5)
        .await;
    let create = |category: &'static str| {
        let (app, token, store) = (&app, shop.seller_token.clone(), shop.store_id);
        async move {
            app.request(
                Method::POST,
                &format!("/stores/{store}/products"),
                Some(product_body(category)),
                Some(&token),
            )
            .await
        }
    };
    for bad in ["Phones", "nope", "FASHION", ""] {
        let res = create(bad).await;
        assert_eq!(res.status, 422, "{bad}: {}", res.json);
        let code = res.json["error"]["code"].as_str().unwrap();
        assert!(
            code == "unknown_category" || code == "validation_failed",
            "{bad}: {}",
            res.json
        );
    }
    let bad = create("nope").await;
    assert_eq!(bad.json["error"]["code"], "unknown_category");
    assert_eq!(create("home-kitchen").await.status, 201);

    // A patch is held to the same list, and keeps the old value when it says nothing.
    let patch = |body: Value| {
        let (app, token, product) = (&app, shop.seller_token.clone(), shop.product_id);
        async move {
            app.request(
                Method::PATCH,
                &format!("/products/{product}"),
                Some(body),
                Some(&token),
            )
            .await
        }
    };
    let res = patch(json!({ "category": "nope" })).await;
    assert_eq!(res.status, 422, "{}", res.json);
    assert_eq!(res.json["error"]["code"], "unknown_category");
    let res = patch(json!({ "category": "electronics" })).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["category"], "electronics");
    let res = patch(json!({ "name": "Renamed" })).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["category"], "electronics");
}

#[sqlx::test]
async fn migration_maps_existing_categories(pool: PgPool) {
    // The migration maps old free text with this function: a slug or a name
    // (any case, any spacing) keeps its category; anything else becomes `other`.
    let map = |raw: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT category_slug_for($1)")
                .bind(raw)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(map("Fashion").await, "fashion");
    assert_eq!(map("  fashion ").await, "fashion");
    assert_eq!(map("HOME AND KITCHEN").await, "home-kitchen");
    assert_eq!(map("home-kitchen").await, "home-kitchen");
    assert_eq!(map("Phones").await, "other");
    assert_eq!(map("").await, "other");
}

#[sqlx::test]
async fn products_cannot_hold_an_unlisted_category_even_by_sql(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = app
        .make_active_store_with_product("08055555551", "Kano Fabrics", 10_000, 5)
        .await;
    let res = sqlx::query("UPDATE products SET category = 'nope' WHERE id = $1")
        .bind(shop.product_id)
        .execute(&app.pool)
        .await;
    assert!(res.is_err(), "the foreign key must hold");
}
