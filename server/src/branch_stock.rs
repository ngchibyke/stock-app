//! Keeps each store's position-0 `branch_stock` rows equal to its variants' counts.
//!
//! Until the stock module (part 7e-1, task 5) makes `branch_stock` the only place stock
//! is written, `product_variants` stays the source and every code path that changes a
//! variant's `stock` or `reserved_quantity` calls [`mirror_to_first_branch`] afterwards,
//! so `variant_availability` stays truthful.

use anyhow::Context;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::AppError;

/// Copies the variants' `stock` and `reserved_quantity` onto their store's position-0
/// branch (creating the row if missing). A variant whose store has no branch is skipped.
/// Call inside the transaction that changed the variants, after the change.
pub async fn mirror_to_first_branch(
    conn: &mut PgConnection,
    variant_ids: &[Uuid],
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO branch_stock (branch_id, variant_id, stock, reserved_quantity) \
         SELECT b.id, v.id, v.stock, v.reserved_quantity \
         FROM product_variants v \
         JOIN products p ON p.id = v.product_id \
         JOIN store_branches b ON b.store_id = p.store_id AND b.position = 0 \
         WHERE v.id = ANY($1) \
         ORDER BY v.id \
         ON CONFLICT (branch_id, variant_id) DO UPDATE \
         SET stock = EXCLUDED.stock, reserved_quantity = EXCLUDED.reserved_quantity",
    )
    .bind(variant_ids)
    .execute(conn)
    .await
    .context("mirror stock to the first branch")?;
    Ok(())
}

/// [`mirror_to_first_branch`] for every variant of one product.
pub async fn mirror_product_to_first_branch(
    conn: &mut PgConnection,
    product_id: Uuid,
) -> Result<(), AppError> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM product_variants WHERE product_id = $1 ORDER BY id")
            .bind(product_id)
            .fetch_all(&mut *conn)
            .await
            .context("load product variants")?;
    mirror_to_first_branch(conn, &ids).await
}
