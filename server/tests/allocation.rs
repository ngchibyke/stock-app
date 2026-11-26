//! Loading the branches that can fill a cart (part 7e-1, task 7).

mod common;

use common::TestApp;
use server::delivery::allocate::load_candidates;
use sqlx::PgPool;
use uuid::Uuid;

const SELLER: &str = "08011111111";

async fn second_branch(app: &TestApp, store_id: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO store_branches (store_id, name, pickup_address_enc, pickup_area_council, position) \
         SELECT store_id, 'Garki', pickup_address_enc, 'bwari', 1 FROM store_branches \
         WHERE store_id = $1 AND position = 0 RETURNING id",
    )
    .bind(store_id)
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

async fn set_cell(app: &TestApp, branch: Uuid, variant: Uuid, stock: i32, reserved: i32) {
    sqlx::query(
        "INSERT INTO branch_stock (branch_id, variant_id, stock, reserved_quantity) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (branch_id, variant_id) \
         DO UPDATE SET stock = $3, reserved_quantity = $4",
    )
    .bind(branch)
    .bind(variant)
    .bind(stock)
    .bind(reserved)
    .execute(&app.pool)
    .await
    .unwrap();
}

#[sqlx::test]
async fn loads_open_branches_with_available_units(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product(SELLER, "Mama Put", 1_000, 4)
        .await;
    let first: Uuid =
        sqlx::query_scalar("SELECT id FROM store_branches WHERE store_id = $1 AND position = 0")
            .bind(l.store_id)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let second = second_branch(&app, l.store_id).await;
    set_cell(&app, first, l.variant_id, 4, 3).await;
    set_cell(&app, second, l.variant_id, 2, 0).await;

    let mut conn = app.pool.acquire().await.unwrap();
    let c = load_candidates(&mut conn, l.store_id, &[l.variant_id])
        .await
        .unwrap();
    assert_eq!(c.len(), 2);
    assert_eq!((c[0].branch_id, c[0].position), (first, 0));
    assert_eq!(c[0].available[&l.variant_id], 1);
    assert_eq!((c[1].branch_id, c[1].position), (second, 1));
    assert_eq!(c[1].available[&l.variant_id], 2);
    assert!(c[1].area.is_some());

    // A variant nobody stocks maps to nothing, not an error.
    let other = Uuid::new_v4();
    let c = load_candidates(&mut conn, l.store_id, &[other])
        .await
        .unwrap();
    assert!(c.iter().all(|c| c.available.is_empty()));
}

#[sqlx::test]
async fn closed_branches_and_deleted_stores_are_left_out(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let l = app
        .make_active_store_with_product(SELLER, "Mama Put", 1_000, 4)
        .await;
    let second = second_branch(&app, l.store_id).await;
    set_cell(&app, second, l.variant_id, 2, 0).await;
    sqlx::query("UPDATE store_branches SET closed_at = now() WHERE id = $1")
        .bind(second)
        .execute(&app.pool)
        .await
        .unwrap();

    let mut conn = app.pool.acquire().await.unwrap();
    let c = load_candidates(&mut conn, l.store_id, &[l.variant_id])
        .await
        .unwrap();
    assert_eq!(c.len(), 1);
    assert_ne!(c[0].branch_id, second);

    sqlx::query("UPDATE stores SET deleted_at = now() WHERE id = $1")
        .bind(l.store_id)
        .execute(&app.pool)
        .await
        .unwrap();
    let c = load_candidates(&mut conn, l.store_id, &[l.variant_id])
        .await
        .unwrap();
    assert!(c.is_empty());
}
