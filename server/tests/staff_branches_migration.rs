//! Migration 0044 on a database that already holds data: every store gets one branch
//! carrying its pickup details, and its stock, orders and delivery options move to it.
//! Runs on a scratch database it creates and drops, so no other test sees it.

use sqlx::{
    Connection, Executor, PgConnection, Row,
    migrate::Migrator,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{path::Path, str::FromStr};
use uuid::Uuid;

#[tokio::test]
async fn migrated_store_has_one_branch_with_identical_stock() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let base = PgConnectOptions::from_str(&url).unwrap();
    let scratch = format!("staff_branches_{}", Uuid::new_v4().simple());

    let mut admin = PgConnection::connect_with(&base).await.unwrap();
    admin
        .execute(format!("CREATE DATABASE {scratch}").as_str())
        .await
        .unwrap();

    let result = tokio::spawn(run(base.clone().database(&scratch))).await;

    admin
        .execute(format!("DROP DATABASE {scratch} WITH (FORCE)").as_str())
        .await
        .unwrap();
    result.unwrap();
}

async fn run(opts: PgConnectOptions) {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(opts)
        .await
        .unwrap();

    let all = Migrator::new(Path::new("./migrations")).await.unwrap();
    let mut old = Migrator::new(Path::new("./migrations")).await.unwrap();
    old.migrations = all
        .migrations
        .iter()
        .filter(|m| m.version <= 43)
        .cloned()
        .collect();
    old.run(&pool).await.unwrap();

    // Data as it stands before branches exist.
    let user: Uuid = sqlx::query_scalar(
        "INSERT INTO users (phone_enc, phone_idx, display_name, password_hash) \
         VALUES (decode('01', 'hex'), decode('02', 'hex'), 'Seller', 'x') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let store: Uuid = sqlx::query_scalar(
        "INSERT INTO stores (owner_id, name, slug, pickup_address_enc, pickup_state, \
            delivery_courier, delivery_self, status, pickup_area_council, pickup_postcode, \
            pickup_courier_address_code, business_phone_enc, self_delivery_fee_kobo) \
         VALUES ($1, 'Shop One', 'shop-one', decode('aabb', 'hex'), 'FCT', true, true, 'active', \
            'bwari', 'FC01ABCDEFG', 4242, decode('ccdd', 'hex'), 50000) RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    let product: Uuid = sqlx::query_scalar(
        "INSERT INTO products (store_id, name, description, category, condition, photo_keys, \
            weight_grams, base_price_kobo) \
         VALUES ($1, 'Thing', 'd', 'other', 'new', ARRAY['k'], 100, 1000) RETURNING id",
    )
    .bind(store)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut variants = Vec::new();
    for (attrs, stock, reserved) in [("{\"s\":\"a\"}", 7, 2), ("{\"s\":\"b\"}", 3, 0)] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO product_variants (product_id, attributes, stock, reserved_quantity) \
             VALUES ($1, $2::jsonb, $3, $4) RETURNING id",
        )
        .bind(product)
        .bind(attrs)
        .bind(stock)
        .bind(reserved)
        .fetch_one(&pool)
        .await
        .unwrap();
        variants.push(id);
    }
    let checkout: Uuid = sqlx::query_scalar(
        "INSERT INTO checkouts (buyer_id, reference, method, provider, total_kobo, status, \
            expires_at) VALUES ($1, 'CK1', 'card', 'fake', 2000, 'paid', now()) RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    let order: Uuid = sqlx::query_scalar(
        "INSERT INTO orders (checkout_id, store_id, buyer_id, status, items_total_kobo, \
            platform_cut_bps, platform_cut_kobo) \
         VALUES ($1, $2, $3, 'paid', 2000, 0, 0) RETURNING id",
    )
    .bind(checkout)
    .bind(store)
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    let address: Uuid = sqlx::query_scalar(
        "INSERT INTO buyer_addresses (user_id, recipient_name_enc, phone_enc, address_line_enc, \
            area_council) VALUES ($1, decode('01', 'hex'), decode('01', 'hex'), decode('01', 'hex'), 'amac') RETURNING id",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    let option: Uuid = sqlx::query_scalar(
        "INSERT INTO delivery_options (buyer_id, store_id, address_id, method, fee_kobo, \
            cart_fingerprint, promised_date, expires_at) \
         VALUES ($1, $2, $3, 'self', 0, repeat('a', 64), current_date, now()) RETURNING id",
    )
    .bind(user)
    .bind(store)
    .bind(address)
    .fetch_one(&pool)
    .await
    .unwrap();

    all.run(&pool).await.unwrap();

    let branches = sqlx::query(
        "SELECT id, name, pickup_address_enc, pickup_area_council::text AS area, \
                pickup_postcode, business_phone_enc, self_delivery_fee_kobo, \
                pickup_courier_address_code, collect_enabled, position, closed_at \
         FROM store_branches WHERE store_id = $1",
    )
    .bind(store)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(branches.len(), 1);
    let b = &branches[0];
    let branch: Uuid = b.get("id");
    assert_eq!(b.get::<String, _>("name"), "Shop One");
    assert_eq!(b.get::<Vec<u8>, _>("pickup_address_enc"), vec![0xaa, 0xbb]);
    assert_eq!(b.get::<String, _>("area"), "bwari");
    assert_eq!(b.get::<String, _>("pickup_postcode"), "FC01ABCDEFG");
    assert_eq!(b.get::<Vec<u8>, _>("business_phone_enc"), vec![0xcc, 0xdd]);
    assert_eq!(b.get::<i64, _>("self_delivery_fee_kobo"), 50000);
    assert_eq!(b.get::<i64, _>("pickup_courier_address_code"), 4242);
    assert!(!b.get::<bool, _>("collect_enabled"));
    assert_eq!(b.get::<i32, _>("position"), 0);
    assert!(
        b.get::<Option<time::OffsetDateTime>, _>("closed_at")
            .is_none()
    );

    for (variant, stock, reserved) in [(variants[0], 7, 2), (variants[1], 3, 0)] {
        let row = sqlx::query(
            "SELECT stock, reserved_quantity FROM branch_stock \
             WHERE branch_id = $1 AND variant_id = $2",
        )
        .bind(branch)
        .bind(variant)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.get::<i32, _>("stock"), stock);
        assert_eq!(row.get::<i32, _>("reserved_quantity"), reserved);
    }

    let order_branch: Uuid = sqlx::query_scalar("SELECT branch_id FROM orders WHERE id = $1")
        .bind(order)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(order_branch, branch);
    let option_branch: Uuid =
        sqlx::query_scalar("SELECT branch_id FROM delivery_options WHERE id = $1")
            .bind(option)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(option_branch, branch);

    for (variant, max) in [(variants[0], 5), (variants[1], 3)] {
        let row = sqlx::query(
            "SELECT total_available, max_available FROM variant_availability \
             WHERE variant_id = $1",
        )
        .bind(variant)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.get::<i64, _>("max_available"), max);
        assert_eq!(row.get::<i64, _>("total_available"), max);
    }
}
