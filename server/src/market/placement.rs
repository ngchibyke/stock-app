//! Puts active stores on plots of the market map. Placements within one district
//! are serialised by a transaction-scoped advisory lock; the UNIQUE `plot_id`
//! column is the backstop. Lock order: users, stores, store_plots, products, ...
//! Callers hold the store row already. `place` still takes `FOR SHARE` on the store
//! row (a soft delete takes NO KEY UPDATE, which conflicts, so a deleted store is
//! never placed); `release` picks the waiting store with `FOR SHARE ... SKIP LOCKED`,
//! so a store being deleted or edited elsewhere is passed over rather than waited
//! for, while one that is merely read-locked keeps its turn.
//! A caller that releases in one district and places in another must first take
//! both district locks with `lock_districts`. Audit details carry ids only.

use anyhow::Context;
use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::layout::Layout;
use crate::{audit, error::AppError};

async fn lock_district(tx: &mut Transaction<'_, Postgres>, district: &str) -> Result<(), AppError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('market_district:' || $1))")
        .bind(district)
        .execute(&mut **tx)
        .await
        .context("lock market district")?;
    Ok(())
}

/// Takes the advisory locks of several districts in sorted id order, so callers that
/// need more than one cannot deadlock against each other. The locks are re-entrant
/// within the transaction and last until it ends.
pub async fn lock_districts(
    tx: &mut Transaction<'_, Postgres>,
    districts: &[&str],
) -> Result<(), AppError> {
    let mut sorted: Vec<&str> = districts.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for district in sorted {
        lock_district(tx, district).await?;
    }
    Ok(())
}

/// Gives an active, non-deleted store the first free plot of its district, in the
/// layout's file order. Idempotent: a store that already has a plot keeps it.
/// `None` when the store is not eligible or its district is full (then
/// `market.district_full` is audited and the store stays active and unplaced).
pub async fn place(
    tx: &mut Transaction<'_, Postgres>,
    layout: &Layout,
    store_id: Uuid,
) -> Result<Option<String>, AppError> {
    let store: Option<(String,)> = sqlx::query_as(
        "SELECT district FROM stores \
         WHERE id = $1 AND status = 'active' AND deleted_at IS NULL FOR SHARE",
    )
    .bind(store_id)
    .fetch_optional(&mut **tx)
    .await
    .context("load store for placement")?;
    let Some((district,)) = store else {
        return Ok(None);
    };

    lock_district(tx, &district).await?;

    let held: Option<String> =
        sqlx::query_scalar("SELECT plot_id FROM store_plots WHERE store_id = $1")
            .bind(store_id)
            .fetch_optional(&mut **tx)
            .await
            .context("load held plot")?;
    if held.is_some() {
        return Ok(held);
    }

    let district_plots: Vec<String> = layout.plots_in(&district).map(|p| p.id.clone()).collect();
    let taken: Vec<String> =
        sqlx::query_scalar("SELECT plot_id FROM store_plots WHERE plot_id = ANY($1)")
            .bind(&district_plots)
            .fetch_all(&mut **tx)
            .await
            .context("load taken plots")?;
    let free = layout
        .plots_in(&district)
        .find(|p| !taken.contains(&p.id))
        .map(|p| p.id.clone());

    let Some(plot) = free else {
        audit::record(
            tx,
            None,
            "market.district_full",
            "store",
            Some(store_id),
            json!({ "district": district }),
        )
        .await?;
        return Ok(None);
    };

    sqlx::query("INSERT INTO store_plots (store_id, plot_id) VALUES ($1, $2)")
        .bind(store_id)
        .bind(&plot)
        .execute(&mut **tx)
        .await
        .context("insert store plot")?;
    audit::record(
        tx,
        None,
        "market.store_placed",
        "store",
        Some(store_id),
        json!({ "plot": plot, "district": district }),
    )
    .await?;
    Ok(Some(plot))
}

/// Frees the store's plot (audit `market.store_unplaced`) and hands it to the oldest
/// active, non-deleted, unplaced store of the same district.
pub async fn release(
    tx: &mut Transaction<'_, Postgres>,
    layout: &Layout,
    store_id: Uuid,
) -> Result<(), AppError> {
    let plot: Option<String> =
        sqlx::query_scalar("SELECT plot_id FROM store_plots WHERE store_id = $1")
            .bind(store_id)
            .fetch_optional(&mut **tx)
            .await
            .context("load plot to release")?;
    let Some(plot) = plot else {
        return Ok(());
    };
    // The plot's own district: the store's column may already have changed.
    let district = match layout.plot(&plot) {
        Some(p) => p.district.clone(),
        None => sqlx::query_scalar("SELECT district FROM stores WHERE id = $1")
            .bind(store_id)
            .fetch_one(&mut **tx)
            .await
            .context("load district")?,
    };
    lock_district(tx, &district).await?;

    let deleted = sqlx::query("DELETE FROM store_plots WHERE store_id = $1")
        .bind(store_id)
        .execute(&mut **tx)
        .await
        .context("delete store plot")?
        .rows_affected();
    if deleted == 0 {
        return Ok(());
    }
    audit::record(
        tx,
        None,
        "market.store_unplaced",
        "store",
        Some(store_id),
        json!({ "plot": plot, "district": district }),
    )
    .await?;

    let next: Option<Uuid> = sqlx::query_scalar(
        "SELECT s.id FROM stores s \
         WHERE s.district = $1 AND s.status = 'active' AND s.deleted_at IS NULL \
           AND s.id <> $2 \
           AND NOT EXISTS (SELECT 1 FROM store_plots sp WHERE sp.store_id = s.id) \
         ORDER BY s.created_at, s.id LIMIT 1 FOR SHARE OF s SKIP LOCKED",
    )
    .bind(&district)
    .bind(store_id)
    .fetch_optional(&mut **tx)
    .await
    .context("find waiting store")?;
    if let Some(next) = next {
        place(tx, layout, next).await?;
    }
    Ok(())
}

/// Places every active, non-deleted store that has no plot, oldest first, one
/// transaction per store. Returns how many got a plot.
pub async fn place_missing(db: &PgPool, layout: &Layout) -> Result<u64, AppError> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT s.id FROM stores s \
         WHERE s.status = 'active' AND s.deleted_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM store_plots sp WHERE sp.store_id = s.id) \
         ORDER BY s.created_at, s.id",
    )
    .fetch_all(db)
    .await
    .context("list unplaced stores")?;
    let mut placed = 0;
    for id in ids {
        let mut tx = db.begin().await.context("begin placement")?;
        if place(&mut tx, layout, id).await?.is_some() {
            placed += 1;
        }
        tx.commit().await.context("commit placement")?;
    }
    Ok(placed)
}
