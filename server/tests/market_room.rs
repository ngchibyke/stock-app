mod common;

use axum::http::Method;
use common::{Listing, TestApp};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn listing(app: &TestApp, n: usize) -> Listing {
    let phone = format!("0803300{n:04}");
    app.make_active_store_with_product(&phone, &format!("Room Shop {n}"), 1000, 5)
        .await
}

async fn slug_of(app: &TestApp, store: Uuid) -> String {
    sqlx::query_scalar("SELECT slug FROM stores WHERE id = $1")
        .bind(store)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn room(app: &TestApp, slug: &str, token: Option<&str>) -> common::TestResponse {
    app.request(Method::GET, &format!("/stores/{slug}/room"), None, token)
        .await
}

async fn age(app: &TestApp, product: Uuid, secs: i64) {
    sqlx::query("UPDATE products SET created_at = now() - make_interval(secs => $2) WHERE id = $1")
        .bind(product)
        .bind(secs as f64)
        .execute(&app.pool)
        .await
        .unwrap();
}

/// A product made straight in the database, `secs` old: quick for filling shelves.
async fn insert_product(
    app: &TestApp,
    store: Uuid,
    name: &str,
    secs: i64,
    published: bool,
    stock: i32,
) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO products (store_id, name, description, category, condition, photo_keys, \
                               weight_grams, base_price_kobo, is_published, created_at) \
         VALUES ($1, $2, 'd', 'fashion', 'new', ARRAY['u/product/x.jpg'], 100, 5000, $4, \
                 now() - make_interval(secs => $3)) RETURNING id",
    )
    .bind(store)
    .bind(name)
    .bind(secs as f64)
    .bind(published)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO product_variants (product_id, attributes, stock) VALUES ($1, '{}'::jsonb, $2)",
    )
    .bind(id)
    .bind(stock)
    .execute(&app.pool)
    .await
    .unwrap();
    id
}

fn placed(res: &common::TestResponse) -> &Vec<Value> {
    res.json["placed"].as_array().expect("placed array")
}

fn names(res: &common::TestResponse) -> Vec<&str> {
    placed(res)
        .iter()
        .map(|p| p["product"]["name"].as_str().unwrap())
        .collect()
}

async fn count(app: &TestApp, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn room_has_the_contract_shape(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;

    let res = room(&app, &slug, None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    let top = res.json.as_object().unwrap();
    assert_eq!(top.len(), 4, "{top:?}");
    for key in ["store", "store_type", "placed", "has_more"] {
        assert!(top.contains_key(key), "missing {key}");
    }
    assert_eq!(res.json["has_more"], false);

    let card = res.json["store"].as_object().unwrap();
    for key in [
        "id",
        "slug",
        "name",
        "logo_url",
        "rating_average",
        "rating_count",
        "product_count",
    ] {
        assert!(card.contains_key(key), "store card is missing {key}");
    }
    assert_eq!(card["slug"], slug.as_str());
    // No personal data rides along.
    for key in ["owner_id", "pickup_address", "business_phone"] {
        assert!(!card.contains_key(key), "store card leaks {key}");
    }

    let boutique = app.state.market.room_type("boutique").unwrap();
    assert_eq!(res.json["store_type"], boutique.json);

    let list = placed(&res);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["slot"], boutique.slots[0].id.as_str());
    let product = list[0]["product"].as_object().unwrap();
    for key in [
        "id",
        "name",
        "category",
        "condition",
        "price_from_kobo",
        "photo_key",
        "photo_url",
        "in_stock",
        "store",
    ] {
        assert!(product.contains_key(key), "product card is missing {key}");
    }
    assert_eq!(product["id"], l.product_id.to_string().as_str());
    assert_eq!(product["in_stock"], true);
    assert_eq!(product["price_from_kobo"], 1000);
    let brief = product["store"].as_object().unwrap();
    for key in ["id", "slug", "name", "rating_average", "rating_count"] {
        assert!(brief.contains_key(key), "store brief is missing {key}");
    }
    assert_eq!(
        res.headers.get("cache-control").unwrap(),
        "public, max-age=30"
    );
}

#[sqlx::test]
async fn placed_products_fill_slots_in_order_newest_first(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    age(&app, l.product_id, 300).await;
    insert_product(&app, l.store_id, "Newest", 10, true, 3).await;
    insert_product(&app, l.store_id, "Middle", 100, true, 3).await;
    let slug = slug_of(&app, l.store_id).await;

    let res = room(&app, &slug, None).await;
    assert_eq!(names(&res), vec!["Newest", "Middle", "Ankara Shirt"]);
    let boutique = app.state.market.room_type("boutique").unwrap();
    let slots: Vec<&str> = placed(&res)
        .iter()
        .map(|p| p["slot"].as_str().unwrap())
        .collect();
    let expected: Vec<&str> = boutique.slots[..3].iter().map(|s| s.id.as_str()).collect();
    assert_eq!(slots, expected);
}

#[sqlx::test]
async fn unpublished_deleted_and_sold_out_products_are_left_out(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    age(&app, l.product_id, 500).await;
    insert_product(&app, l.store_id, "Draft", 10, false, 3).await;
    let gone = insert_product(&app, l.store_id, "Gone", 20, true, 3).await;
    sqlx::query("UPDATE products SET deleted_at = now() WHERE id = $1")
        .bind(gone)
        .execute(&app.pool)
        .await
        .unwrap();
    insert_product(&app, l.store_id, "Sold out", 30, true, 0).await;
    insert_product(&app, l.store_id, "Fine", 40, true, 2).await;
    let slug = slug_of(&app, l.store_id).await;

    let res = room(&app, &slug, None).await;
    assert_eq!(names(&res), vec!["Fine", "Ankara Shirt"]);
    assert_eq!(res.json["has_more"], false);
}

#[sqlx::test]
async fn a_unit_reserved_to_zero_leaves_the_shelf(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;
    assert_eq!(names(&room(&app, &slug, None).await).len(), 1);

    sqlx::query("UPDATE product_variants SET reserved_quantity = stock WHERE id = $1")
        .bind(l.variant_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = room(&app, &slug, None).await;
    assert!(placed(&res).is_empty(), "{}", res.json);

    sqlx::query("UPDATE product_variants SET reserved_quantity = stock - 1 WHERE id = $1")
        .bind(l.variant_id)
        .execute(&app.pool)
        .await
        .unwrap();
    assert_eq!(placed(&room(&app, &slug, None).await).len(), 1);
}

#[sqlx::test]
async fn more_products_than_slots_sets_has_more_and_caps_placed(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    age(&app, l.product_id, 10_000).await;
    for i in 0..9 {
        insert_product(&app, l.store_id, &format!("P{i}"), 10 + i, true, 1).await;
    }
    let slug = slug_of(&app, l.store_id).await;
    let slots = app.state.market.room_type("boutique").unwrap().slots.len();
    assert_eq!(slots, 9);

    // Ten qualify, nine slots: the oldest stays in the back room.
    let res = room(&app, &slug, None).await;
    assert_eq!(placed(&res).len(), 9);
    assert_eq!(res.json["has_more"], true);
    assert!(!names(&res).contains(&"Ankara Shirt"));

    // Exactly nine: nothing left over.
    sqlx::query("UPDATE products SET deleted_at = now() WHERE id = $1")
        .bind(l.product_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let res = room(&app, &slug, None).await;
    assert_eq!(placed(&res).len(), 9);
    assert_eq!(res.json["has_more"], false);
}

#[sqlx::test]
async fn a_store_with_no_products_has_an_empty_room(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    sqlx::query("UPDATE products SET is_published = false WHERE store_id = $1")
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let slug = slug_of(&app, l.store_id).await;

    let res = room(&app, &slug, None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["placed"], json!([]));
    assert_eq!(res.json["has_more"], false);
    assert!(res.json["store_type"]["slots"].is_array());
}

#[sqlx::test]
async fn a_product_with_no_photo_still_appears(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;
    // No public photo store is set up: no photo has an address.
    app.use_no_public_photos();

    let res = room(&app, &slug, None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(placed(&res).len(), 1);
    assert!(placed(&res)[0]["product"]["photo_url"].is_null());
    assert!(res.json["store"]["logo_url"].is_null());
}

#[sqlx::test]
async fn suspended_store_room_is_404_even_for_the_owner(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;
    sqlx::query("UPDATE stores SET status = 'suspended' WHERE id = $1")
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();

    for token in [None, Some(l.seller_token.as_str())] {
        let res = room(&app, &slug, token).await;
        assert_eq!(res.status, 404, "{}", res.json);
        assert_eq!(res.json["error"]["code"], "store_not_found");
    }
}

#[sqlx::test]
async fn pending_store_room_is_visible_to_the_owner_only(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;
    sqlx::query("UPDATE stores SET status = 'pending' WHERE id = $1")
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let (_, other) = app.make_seller("08033009999").await;

    let res = room(&app, &slug, Some(&l.seller_token)).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(placed(&res).len(), 1);
    assert_eq!(
        res.headers.get("cache-control").unwrap(),
        "private, no-store"
    );

    let unknown = room(&app, "no-such-store-anywhere", None).await;
    for token in [None, Some(other.as_str())] {
        let res = room(&app, &slug, token).await;
        assert_eq!(res.status, 404, "{}", res.json);
        assert_eq!(res.json["error"]["code"], "store_not_found");
        assert_eq!(res.json, unknown.json);
    }

    // Bad credentials are still refused, as elsewhere.
    let res = room(&app, &slug, Some("not-a-token")).await;
    assert_eq!(res.status, 401, "{}", res.json);
}

#[sqlx::test]
async fn deleted_and_unknown_slugs_are_404_with_the_same_body(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;
    sqlx::query("UPDATE stores SET deleted_at = now() WHERE id = $1")
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();

    let deleted = room(&app, &slug, Some(&l.seller_token)).await;
    let unknown = room(&app, "never-existed", None).await;
    assert_eq!(deleted.status, 404);
    assert_eq!(unknown.status, 404);
    assert_eq!(deleted.json["error"]["code"], "store_not_found");
    assert_eq!(deleted.json, unknown.json);
}

#[sqlx::test]
async fn room_type_follows_the_district(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;
    sqlx::query("UPDATE stores SET district = 'phones-electronics' WHERE id = $1")
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();

    let res = room(&app, &slug, None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["store_type"]["id"], "phone_shop");
    let phone = app.state.market.room_type("phone_shop").unwrap();
    assert_eq!(res.json["store_type"], phone.json);
    assert_eq!(placed(&res)[0]["slot"], phone.slots[0].id.as_str());
}

#[sqlx::test]
async fn an_unknown_district_falls_back_to_the_everything_else_room(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;
    sqlx::query("UPDATE stores SET district = 'retired-district' WHERE id = $1")
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();

    let res = room(&app, &slug, None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(res.json["store_type"]["id"], "boutique");
}

#[sqlx::test]
async fn room_needs_no_login_for_an_active_store(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;

    let res = room(&app, &slug, None).await;
    assert_eq!(res.status, 200, "{}", res.json);
    assert_eq!(placed(&res).len(), 1);
}

#[sqlx::test]
async fn editing_a_price_does_not_reorder_the_shelf(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    age(&app, l.product_id, 500).await;
    insert_product(&app, l.store_id, "Newer", 100, true, 3).await;
    let slug = slug_of(&app, l.store_id).await;
    let before = room(&app, &slug, None).await;
    assert_eq!(names(&before), vec!["Newer", "Ankara Shirt"]);

    let res = app
        .request(
            Method::PATCH,
            &format!("/products/{}", l.product_id),
            Some(json!({ "base_price_kobo": 2500 })),
            Some(&l.seller_token),
        )
        .await;
    assert_eq!(res.status, 200, "{}", res.json);

    let after = room(&app, &slug, None).await;
    assert_eq!(names(&after), vec!["Newer", "Ankara Shirt"]);
    assert_eq!(placed(&after)[1]["product"]["price_from_kobo"], 2500);
}

#[sqlx::test]
async fn room_read_writes_nothing(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = listing(&app, 1).await;
    let slug = slug_of(&app, l.store_id).await;
    let audit = count(&app, "audit_log").await;
    let plots = count(&app, "store_plots").await;

    for token in [None, Some(l.seller_token.as_str())] {
        assert_eq!(room(&app, &slug, token).await.status, 200);
    }
    assert_eq!(room(&app, "nope", None).await.status, 404);

    assert_eq!(count(&app, "audit_log").await, audit);
    assert_eq!(count(&app, "store_plots").await, plots);
}
