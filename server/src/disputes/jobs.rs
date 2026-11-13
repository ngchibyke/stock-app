//! Job 12, `dispute_deadlines`: a dispute still `awaiting_seller` past its
//! stored `seller_respond_by` moves on to `awaiting_decision` with
//! `seller_missed_deadline = true`. Each row is handled in its own short
//! transaction: the order is locked first (global lock order), then the
//! dispute, both `FOR UPDATE SKIP LOCKED`, and the deadline is re-checked
//! under the lock so a seller's response racing the job wins or loses cleanly.

use anyhow::Context;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    AppState,
    admin::LOCK_TIMEOUT,
    audit,
    error::AppError,
    escrow::jobs::{BATCH, Stats},
};

/// Job name (`job_runs.job`).
pub const DISPUTE_DEADLINES: &str = "dispute_deadlines";

pub async fn run(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    let due: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, order_id FROM disputes \
         WHERE status = 'awaiting_seller' AND seller_respond_by <= now() \
         ORDER BY seller_respond_by, id LIMIT $1",
    )
    .bind(BATCH)
    .fetch_all(&state.db)
    .await
    .context("find due disputes")?;
    let mut stats = Stats::default();
    for (dispute_id, order_id) in due {
        if cancel.is_cancelled() {
            break;
        }
        match advance(state, dispute_id, order_id).await {
            Ok(true) => stats.done += 1,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(job = DISPUTE_DEADLINES, error = %e, "dispute not advanced");
                stats.failed += 1;
            }
        }
    }
    Ok(stats)
}

async fn advance(state: &AppState, dispute_id: Uuid, order_id: Uuid) -> Result<bool, AppError> {
    let mut tx = state.db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    let order: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM orders WHERE id = $1 FOR UPDATE SKIP LOCKED")
            .bind(order_id)
            .fetch_optional(&mut *tx)
            .await
            .context("lock order")?;
    if order.is_none() {
        return Ok(false);
    }
    let moved: Option<Uuid> = sqlx::query_scalar(
        "UPDATE disputes SET status = 'awaiting_decision', seller_missed_deadline = true \
         WHERE id = (SELECT id FROM disputes WHERE id = $1 AND status = 'awaiting_seller' \
                       AND seller_respond_by <= now() FOR UPDATE SKIP LOCKED) \
         RETURNING id",
    )
    .bind(dispute_id)
    .fetch_optional(&mut *tx)
    .await
    .context("move dispute on")?;
    if moved.is_none() {
        return Ok(false);
    }
    audit::record(
        &mut tx,
        None,
        "dispute.seller_missed_deadline",
        "dispute",
        Some(dispute_id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(true)
}
