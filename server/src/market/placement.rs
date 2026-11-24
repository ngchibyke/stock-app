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
        // Audited once per wait: not again while the store's latest market row
        // already says the same district was full.
        let last: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT action, detail->>'district' FROM audit_log              WHERE subject_type = 'store' AND subject_id = $1 AND action LIKE 'market.%'              ORDER BY id DESC LIMIT 1",
        )
        .bind(store_id)
        .fetch_optional(&mut **tx)
        .await
        .context("load last market audit")?;
        let already = last.is_some_and(|(action, d)| {
            action == "market.district_full" && d.as_deref() == Some(district.as_str())
        });
        if !already {
            audit::record(
                tx,
                None,
                "market.district_full",
                "store",
                Some(store_id),
                json!({ "district": district }),
            )
            .await?;
        }
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

/// Drops every `store_plots` row the layout cannot honour: the plot id is not in the
/// layout, or the plot sits in another district than the store's current one (a layout
/// swap renamed, removed or moved it). Audited as `market.store_unplaced`; the store
/// is placed again by `place_missing`. One transaction per row; the store row comes
/// first, then the district locks. Returns how many rows were dropped.
async fn release_stale(db: &PgPool, layout: &Layout) -> Result<u64, AppError> {
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT sp.store_id, sp.plot_id, s.district          FROM store_plots sp JOIN stores s ON s.id = sp.store_id",
    )
    .fetch_all(db)
    .await
    .context("list held plots")?;
    let mut dropped = 0;
    for (store_id, plot, district) in rows {
        let plot_district = layout.plot(&plot).map(|p| p.district.as_str());
        if plot_district == Some(district.as_str()) {
            continue;
        }
        let mut tx = db.begin().await.context("begin stale release")?;
        // Re-read under the store row: the store may have moved on since the list.
        let current: Option<(String, String)> = sqlx::query_as(
            "SELECT sp.plot_id, s.district FROM stores s              JOIN store_plots sp ON sp.store_id = s.id WHERE s.id = $1 FOR SHARE OF s",
        )
        .bind(store_id)
        .fetch_optional(&mut *tx)
        .await
        .context("reload held plot")?;
        let Some((plot, district)) = current else {
            continue;
        };
        let plot_district = layout.plot(&plot).map(|p| p.district.clone());
        if plot_district.as_deref() == Some(district.as_str()) {
            continue;
        }
        let mut locks = vec![district.as_str()];
        locks.extend(plot_district.as_deref());
        lock_districts(&mut tx, &locks).await?;
        let deleted = sqlx::query("DELETE FROM store_plots WHERE store_id = $1 AND plot_id = $2")
            .bind(store_id)
            .bind(&plot)
            .execute(&mut *tx)
            .await
            .context("delete stale plot")?
            .rows_affected();
        if deleted == 1 {
            audit::record(
                &mut tx,
                None,
                "market.store_unplaced",
                "store",
                Some(store_id),
                json!({ "plot": plot, "district": district, "reason": "layout_changed" }),
            )
            .await?;
            dropped += 1;
        }
        tx.commit().await.context("commit stale release")?;
    }
    Ok(dropped)
}

/// First drops plots the layout no longer honours (`release_stale`), then places
/// every active, non-deleted store that has no plot, oldest first, one transaction
/// per store. A store that fails is logged (its id only) and skipped, so one bad
/// row cannot stop the rest. Returns how many got a plot.
pub async fn place_missing(db: &PgPool, layout: &Layout) -> Result<u64, AppError> {
    let dropped = release_stale(db, layout).await?;
    if dropped > 0 {
        tracing::info!(dropped, "stale market plots released");
    }
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT s.id FROM stores s          WHERE s.status = 'active' AND s.deleted_at IS NULL            AND NOT EXISTS (SELECT 1 FROM store_plots sp WHERE sp.store_id = s.id)          ORDER BY s.created_at, s.id",
    )
    .fetch_all(db)
    .await
    .context("list unplaced stores")?;
    let mut placed = 0;
    for id in ids {
        match place_one(db, layout, id).await {
            Ok(true) => placed += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(store_id = %id, error = %e, "store not placed; continuing"),
        }
    }
    Ok(placed)
}

async fn place_one(db: &PgPool, layout: &Layout, id: Uuid) -> Result<bool, AppError> {
    let mut tx = db.begin().await.context("begin placement")?;
    let got = place(&mut tx, layout, id).await?.is_some();
    tx.commit().await.context("commit placement")?;
    Ok(got)
}
