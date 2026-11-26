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
            let body = app
                .ready_photos_in(&token, "product", product_body(category))
                .await;
            app.request(
                Method::POST,
                &format!("/stores/{store}/products"),
                Some(body),
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

// ---- Browse and search (task 2) ----

use common::{Listing, TestResponse};
use uuid::Uuid;

/// Inserts a published product with one variant straight into the database, so
/// tests can make hundreds quickly and control `created_at`.
async fn add(
    app: &TestApp,
    store: Uuid,
    name: &str,
    category: &str,
    price: i64,
    age_minutes: i32,
) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO products (store_id, name, description, category, brand, condition, \
                photo_keys, weight_grams, base_price_kobo, is_published, created_at) \
         VALUES ($1, $2, 'd', $3, NULL, 'new', ARRAY['photos/a.jpg'], 300, $4, true, \
                 now() - make_interval(mins => $5)) RETURNING id",
    )
    .bind(store)
    .bind(name)
    .bind(category)
    .bind(price)
    .bind(age_minutes)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let variant: Uuid = sqlx::query_scalar(
        "INSERT INTO product_variants (product_id, attributes, stock) \
         VALUES ($1, '{}'::jsonb, 0) RETURNING id",
    )
    .bind(id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    app.set_counts(variant, 5, 0).await;
    id
}

/// An active store whose starter product is unpublished, so a test controls
/// exactly what is listed.
async fn bare_shop(app: &TestApp, phone: &str, name: &str) -> Listing {
    let shop = app
        .make_active_store_with_product(phone, name, 10_000, 5)
        .await;
    sqlx::query("UPDATE products SET is_published = false WHERE id = $1")
        .bind(shop.product_id)
        .execute(&app.pool)
        .await
        .unwrap();
    shop
}

async fn browse(app: &TestApp, query: &str) -> TestResponse {
    app.request(Method::GET, &format!("/products{query}"), None, None)
        .await
}

fn names(res: &TestResponse) -> Vec<String> {
    res.json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect()
}

fn ids(res: &TestResponse) -> Vec<String> {
    res.json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test]
async fn browse_lists_published_products_newest_first(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    sqlx::query("UPDATE stores SET rating_count = 2, rating_sum = 9 WHERE id = $1")
        .bind(shop.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    add(&app, shop.store_id, "Oldest", "fashion", 1_000, 30).await;
    add(&app, shop.store_id, "Middle", "fashion", 2_000, 20).await;
    add(&app, shop.store_id, "Newest", "fashion", 3_000, 10).await;

    let res = browse(&app, "").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(names(&res), ["Newest", "Middle", "Oldest"]);
    assert_eq!(res.json["next_cursor"], Value::Null);
    let card = &res.json["items"][0];
    assert_eq!(card["category"], "fashion");
    assert_eq!(card["condition"], "new");
    assert_eq!(card["price_from_kobo"], 3_000);
    assert_eq!(card["photo_key"], "photos/a.jpg");
    assert_eq!(card["in_stock"], true);
    assert_eq!(card["store"]["name"], "Kano Fabrics");
    assert_eq!(card["store"]["rating_count"], 2);
    assert_eq!(card["store"]["rating_average"], 4.5);
    assert!(card["store"]["slug"].as_str().unwrap().len() > 2);
    // Nothing private about the seller is on a card.
    assert!(card["store"].get("owner_id").is_none());
}

#[sqlx::test]
async fn hidden_products_never_appear(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    add(&app, shop.store_id, "Visible shirt", "fashion", 1_000, 5).await;
    let unpublished = add(&app, shop.store_id, "Draft shirt", "fashion", 1_000, 5).await;
    sqlx::query("UPDATE products SET is_published = false WHERE id = $1")
        .bind(unpublished)
        .execute(&app.pool)
        .await
        .unwrap();
    let deleted = add(&app, shop.store_id, "Deleted shirt", "fashion", 1_000, 5).await;
    sqlx::query("UPDATE products SET deleted_at = now() WHERE id = $1")
        .bind(deleted)
        .execute(&app.pool)
        .await
        .unwrap();
    // A suspended store and a pending store.
    let other = bare_shop(&app, "08066666661", "Garki Bags").await;
    add(&app, other.store_id, "Suspended shirt", "fashion", 1_000, 5).await;
    let third = bare_shop(&app, "08077777771", "Jabi Shoes").await;
    add(&app, third.store_id, "Pending shirt", "fashion", 1_000, 5).await;
    sqlx::query("UPDATE stores SET status = 'suspended' WHERE id = $1")
        .bind(other.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE stores SET status = 'pending' WHERE id = $1")
        .bind(third.store_id)
        .execute(&app.pool)
        .await
        .unwrap();

    assert_eq!(names(&browse(&app, "").await), ["Visible shirt"]);
    assert_eq!(names(&browse(&app, "?q=shirt").await), ["Visible shirt"]);
    assert_eq!(
        names(&browse(&app, "?category=fashion&sort=price_asc").await),
        ["Visible shirt"]
    );
}

#[sqlx::test]
async fn filters_combine(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    add(&app, shop.store_id, "Cheap shirt", "fashion", 1_000, 5).await;
    add(&app, shop.store_id, "Mid shirt", "fashion", 5_000, 5).await;
    add(&app, shop.store_id, "Pricey shirt", "fashion", 9_000, 5).await;
    add(&app, shop.store_id, "Mid phone", "phones-gadgets", 5_000, 5).await;
    let used = add(&app, shop.store_id, "Used mid shirt", "fashion", 5_000, 5).await;
    sqlx::query("UPDATE products SET condition = 'used' WHERE id = $1")
        .bind(used)
        .execute(&app.pool)
        .await
        .unwrap();

    let res = browse(
        &app,
        "?category=fashion&min_price_kobo=2000&max_price_kobo=6000",
    )
    .await;
    let mut got = names(&res);
    got.sort();
    assert_eq!(got, ["Mid shirt", "Used mid shirt"]);
    let res = browse(&app, "?category=fashion&condition=used").await;
    assert_eq!(names(&res), ["Used mid shirt"]);
    let slug: String = sqlx::query_scalar("SELECT slug FROM stores WHERE id = $1")
        .bind(shop.store_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        names(&browse(&app, &format!("?store={slug}&category=phones-gadgets")).await),
        ["Mid phone"]
    );
    assert!(names(&browse(&app, "?store=no-such-store").await).is_empty());

    for bad in [
        "?min_price_kobo=6000&max_price_kobo=2000",
        "?min_price_kobo=0",
        "?condition=broken",
        "?limit=0",
        "?sort=cheapest",
    ] {
        let res = browse(&app, bad).await;
        assert_eq!(res.status, 422, "{bad}: {}", res.json);
    }
    // A page size above the maximum is clamped, not refused.
    assert_eq!(browse(&app, "?limit=500").await.status, 200);
}

#[sqlx::test]
async fn search_finds_close_spellings_and_brands(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    add(&app, shop.store_id, "Leather Sandal", "fashion", 1_000, 5).await;
    let branded = add(&app, shop.store_id, "Running shoe", "fashion", 2_000, 5).await;
    sqlx::query("UPDATE products SET brand = 'Nike' WHERE id = $1")
        .bind(branded)
        .execute(&app.pool)
        .await
        .unwrap();
    add(&app, shop.store_id, "Gas cooker", "home-kitchen", 3_000, 5).await;

    assert_eq!(names(&browse(&app, "?q=sandal").await), ["Leather Sandal"]);
    assert_eq!(names(&browse(&app, "?q=sandle").await), ["Leather Sandal"]);
    assert_eq!(names(&browse(&app, "?q=nike").await), ["Running shoe"]);
    assert_eq!(names(&browse(&app, "?q=SANDAL").await), ["Leather Sandal"]);
    assert_eq!(names(&browse(&app, "?q=cooker").await), ["Gas cooker"]);
    // A category word finds its products too.
    assert_eq!(names(&browse(&app, "?q=home").await), ["Gas cooker"]);
    assert!(names(&browse(&app, "?q=zebra").await).is_empty());
    // The best match comes first.
    let res = browse(&app, "?q=leather%20sandal").await;
    assert_eq!(names(&res)[0], "Leather Sandal");
}

#[sqlx::test]
async fn search_text_is_parameterised(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    add(&app, shop.store_id, "Plain shirt", "fashion", 1_000, 5).await;
    for q in [
        "%27%3B%20DROP%20TABLE%20products%3B%20--",
        "%25",
        "_",
        "%5C",
        "%25%25%25%25",
        "caf%C3%A9%20%E2%98%83%20%E6%97%A5%E6%9C%AC",
        "a%20b%20c%20d%20e%20f%20g%20h%20i%20j",
    ] {
        let res = browse(&app, &format!("?q={q}")).await;
        assert_eq!(res.status, 200, "{q}: {}", res.json);
    }
    // A bare wildcard must not match everything.
    assert!(names(&browse(&app, "?q=%25").await).is_empty());
    assert!(names(&browse(&app, "?q=_").await).is_empty());
    // Too long is refused.
    let res = browse(&app, &format!("?q={}", "a".repeat(81))).await;
    assert_eq!(res.status, 422, "{}", res.json);
    // The table is still there and still holds the product.
    assert_eq!(names(&browse(&app, "").await), ["Plain shirt"]);
}

#[sqlx::test]
async fn pages_are_stable_when_prices_tie(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    let mut all = Vec::new();
    for i in 0..20 {
        all.push(
            add(
                &app,
                shop.store_id,
                &format!("Item {i}"),
                "fashion",
                5_000,
                i,
            )
            .await
            .to_string(),
        );
    }
    all.sort();
    for sort in ["price_asc", "price_desc", "newest"] {
        let mut seen: Vec<String> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..10 {
            let mut query = format!("?sort={sort}&limit=7");
            if let Some(c) = &cursor {
                query.push_str(&format!("&cursor={c}"));
            }
            let res = browse(&app, &query).await;
            assert_eq!(res.status, 200, "{sort}: {}", res.json);
            seen.extend(ids(&res));
            cursor = res.json["next_cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen.len(), 20, "{sort}: every item once");
        let mut sorted = seen.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, all, "{sort}: no repeats, no gaps");
    }
}

#[sqlx::test]
async fn relevance_paging_stops_at_200(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    sqlx::query(
        "INSERT INTO products (store_id, name, description, category, condition, photo_keys, \
                weight_grams, base_price_kobo, is_published) \
         SELECT $1, 'Rice bag ' || g, 'd', 'food-groceries', 'new', ARRAY['photos/a.jpg'], 300, 1000 + g, true \
         FROM generate_series(1, 205) g",
    )
    .bind(shop.store_id)
    .execute(&app.pool)
    .await
    .unwrap();
    let variants: Vec<Uuid> = sqlx::query_scalar(
        "INSERT INTO product_variants (product_id, attributes, stock) \
         SELECT id, '{}'::jsonb, 0 FROM products WHERE name LIKE 'Rice bag %' RETURNING id",
    )
    .fetch_all(&app.pool)
    .await
    .unwrap();
    for v in variants {
        app.set_counts(v, 5, 0).await;
    }
    let mut seen = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        let mut query = "?q=rice&limit=50".to_string();
        if let Some(c) = &cursor {
            query.push_str(&format!("&cursor={c}"));
        }
        let res = browse(&app, &query).await;
        assert_eq!(res.status, 200, "{}", res.json);
        for id in ids(&res) {
            assert!(seen.insert(id), "relevance paging repeated an item");
        }
        pages += 1;
        cursor = res.json["next_cursor"].as_str().map(str::to_string);
        if cursor.is_none() || pages > 10 {
            break;
        }
    }
    assert_eq!(seen.len(), 200, "the cap");
    assert_eq!(pages, 4);
}

#[sqlx::test]
async fn bad_cursor_is_422(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    for i in 0..3 {
        add(
            &app,
            shop.store_id,
            &format!("Item {i}"),
            "fashion",
            1_000 + i64::from(i),
            i,
        )
        .await;
    }
    // A real cursor, from the newest sort, offered to another sort.
    let first = browse(&app, "?limit=1").await;
    let good = first.json["next_cursor"].as_str().unwrap().to_string();
    assert_eq!(
        browse(&app, &format!("?limit=1&cursor={good}"))
            .await
            .status,
        200
    );
    for bad in [
        "not-base64!!".to_string(),
        "Zm9v".to_string(),
        format!("{good}&sort=price_asc"),
    ] {
        let res = browse(&app, &format!("?cursor={bad}")).await;
        assert_eq!(res.status, 422, "{bad}: {}", res.json);
        assert_eq!(res.json["error"]["code"], "bad_cursor");
    }
}

#[sqlx::test]
async fn price_from_uses_variant_overrides_and_in_stock_follows_reservations(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    let id = add(&app, shop.store_id, "Two sizes", "fashion", 1_000, 5).await;
    sqlx::query("DELETE FROM product_variants WHERE product_id = $1")
        .bind(id)
        .execute(&app.pool)
        .await
        .unwrap();
    let variants: Vec<(Uuid, i32)> = sqlx::query_as(
        "INSERT INTO product_variants (product_id, attributes, stock, price_kobo) VALUES \
         ($1, '{\"size\":\"S\"}'::jsonb, 0, 800), ($1, '{\"size\":\"L\"}'::jsonb, 0, NULL) \
         RETURNING id, CASE WHEN price_kobo IS NULL THEN 1 ELSE 2 END",
    )
    .bind(id)
    .fetch_all(&app.pool)
    .await
    .unwrap();
    for &(v, n) in &variants {
        app.set_counts(v, n, 0).await;
    }
    let res = browse(&app, "").await;
    assert_eq!(res.json["items"][0]["price_from_kobo"], 800);
    assert_eq!(res.json["items"][0]["in_stock"], true);
    // The price filter looks at the price a buyer would pay.
    assert_eq!(
        names(&browse(&app, "?max_price_kobo=900").await),
        ["Two sizes"]
    );
    assert!(names(&browse(&app, "?min_price_kobo=900").await).is_empty());

    // Every unit held by pending checkouts: nothing left to buy.
    for &(v, n) in &variants {
        app.set_counts(v, n, n).await;
    }
    assert_eq!(browse(&app, "").await.json["items"][0]["in_stock"], false);
}

#[sqlx::test]
async fn trigram_index_exists(pool: PgPool) {
    let def: String = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes \
         WHERE tablename = 'products' AND indexname = 'products_search_idx'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(def.contains("gin_trgm_ops"), "{def}");
    assert!(def.contains("lower("), "{def}");
}

// ---- Store directory (task 3) ----

async fn directory(app: &TestApp, query: &str) -> TestResponse {
    app.request(Method::GET, &format!("/stores{query}"), None, None)
        .await
}

fn store_names(res: &TestResponse) -> Vec<String> {
    res.json["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test]
async fn directory_lists_only_active_stores(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let kano = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    add(&app, kano.store_id, "Shirt", "fashion", 1_000, 5).await;
    add(&app, kano.store_id, "Cap", "fashion", 1_000, 5).await;
    let draft = add(&app, kano.store_id, "Draft", "fashion", 1_000, 5).await;
    sqlx::query("UPDATE products SET is_published = false WHERE id = $1")
        .bind(draft)
        .execute(&app.pool)
        .await
        .unwrap();
    let garki = bare_shop(&app, "08066666661", "Garki Bags").await;
    let jabi = bare_shop(&app, "08077777771", "Jabi Shoes").await;
    let wuse = bare_shop(&app, "08088888881", "Wuse Phones").await;
    for (id, status) in [(garki.store_id, "suspended"), (jabi.store_id, "pending")] {
        sqlx::query("UPDATE stores SET status = $2::store_status WHERE id = $1")
            .bind(id)
            .bind(status)
            .execute(&app.pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE stores SET deleted_at = now() WHERE id = $1")
        .bind(wuse.store_id)
        .execute(&app.pool)
        .await
        .unwrap();

    let res = directory(&app, "").await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(store_names(&res), ["Kano Fabrics"]);
    let card = &res.json["items"][0];
    assert_eq!(card["product_count"], 2, "only published products count");
    assert_eq!(card["rating_count"], 0);
    assert_eq!(card["rating_average"], Value::Null);
    assert!(card["slug"].as_str().unwrap().len() > 2);
    assert!(card.get("owner_id").is_none());
}

#[sqlx::test]
async fn directory_search_and_rating_sort(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let a = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    let b = bare_shop(&app, "08066666661", "Garki Fabrics").await;
    let c = bare_shop(&app, "08077777771", "Jabi Shoes").await;
    for (id, count, sum) in [
        (a.store_id, 2, 8_i64),
        (b.store_id, 3, 14),
        (c.store_id, 0, 0),
    ] {
        sqlx::query("UPDATE stores SET rating_count = $2, rating_sum = $3 WHERE id = $1")
            .bind(id)
            .bind(count)
            .bind(sum)
            .execute(&app.pool)
            .await
            .unwrap();
    }
    // Garki 4.7, Kano 4.0, Jabi unreviewed: last, not first.
    let res = directory(&app, "?sort=rating").await;
    assert_eq!(
        store_names(&res),
        ["Garki Fabrics", "Kano Fabrics", "Jabi Shoes"]
    );
    assert_eq!(res.json["items"][0]["rating_average"], 4.7);
    assert_eq!(
        store_names(&directory(&app, "?q=FABRICS&sort=rating").await),
        ["Garki Fabrics", "Kano Fabrics"]
    );
    assert_eq!(
        store_names(&directory(&app, "?q=shoe").await),
        ["Jabi Shoes"]
    );
    // A wildcard is text, not a pattern.
    assert!(store_names(&directory(&app, "?q=%25").await).is_empty());
    assert_eq!(directory(&app, "?sort=best").await.status, 422);
    assert_eq!(directory(&app, "?cursor=junk").await.status, 422);
}

#[sqlx::test]
async fn directory_pages_without_repeats(pool: PgPool) {
    let app = TestApp::new(pool).await;
    for i in 0..5 {
        bare_shop(&app, &format!("080{i}0000000"), &format!("Store {i}")).await;
    }
    for sort in ["newest", "rating"] {
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..6 {
            let mut query = format!("?sort={sort}&limit=2");
            if let Some(c) = &cursor {
                query.push_str(&format!("&cursor={c}"));
            }
            let res = directory(&app, &query).await;
            assert_eq!(res.status, 200, "{sort}: {}", res.json);
            seen.extend(store_names(&res));
            cursor = res.json["next_cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 5, "{sort}: every store once");
    }
}

#[sqlx::test]
async fn directory_does_not_shadow_mine_or_slug_routes(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let shop = bare_shop(&app, "08055555551", "Kano Fabrics").await;
    // The seller's own list, the public store page and creating a store all still answer.
    let mine = app
        .request(Method::GET, "/stores/mine", None, Some(&shop.seller_token))
        .await;
    assert_eq!(mine.status, 200, "{}", mine.json);
    let slug: String = sqlx::query_scalar("SELECT slug FROM stores WHERE id = $1")
        .bind(shop.store_id)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let page = app
        .request(Method::GET, &format!("/stores/{slug}"), None, None)
        .await;
    assert_eq!(page.status, 200, "{}", page.json);
    let create = app
        .request(
            Method::POST,
            "/stores",
            Some(json!({})),
            Some(&shop.seller_token),
        )
        .await;
    assert_eq!(
        create.status, 422,
        "an empty body reaches the create handler: {}",
        create.json
    );
}
