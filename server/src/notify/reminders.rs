//! Job 13, `confirm_reminders`: nudges the buyer of a `delivered` order to
//! confirm it or report a problem before the money is released.
//!
//! The "delivered" notice is the first reminder. After it, for an order that
//! is not held and has no payout:
//!
//! - stage 1, `confirm_reminder`, at `delivered_at + 24h`, only if that is
//!   before `release_at - 24h`;
//! - stage 2, `release_tomorrow`, at `release_at - 24h`, only if that is after
//!   `delivered_at`;
//! - nothing at or after `release_at`.
//!
//! A stage whose moment has passed is skipped, never sent late: an order first
//! seen inside its last 24 hours gets `release_tomorrow` alone. Every
//! comparison uses the database clock and the deadlines stored on the order.
//! `orders.confirm_reminders_sent` holds the last stage sent; it is advanced
//! under the order lock, in the transaction that raises the notice.

use anyhow::Context;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    kinds::Kind,
    service::{Notice, enqueue},
};
use crate::{
    AppState,
    admin::LOCK_TIMEOUT,
    error::AppError,
    escrow::jobs::{BATCH, Stats},
};

/// Job name (`job_runs.job`).
pub const CONFIRM_REMINDERS: &str = "confirm_reminders";

/// An order due a reminder now. `o` is the order.
const DUE: &str = "o.status = 'delivered' AND o.held_at IS NULL \
     AND o.delivered_at IS NOT NULL AND o.release_at IS NOT NULL \
     AND now() < o.release_at \
     AND NOT EXISTS (SELECT 1 FROM payouts p WHERE p.order_id = o.id) \
     AND ((o.confirm_reminders_sent = 0 \
           AND now() >= o.delivered_at + interval '24 hours' \
           AND o.delivered_at + interval '24 hours' < o.release_at - interval '24 hours') \
       OR (o.confirm_reminders_sent < 2 \
           AND now() >= o.release_at - interval '24 hours' \
           AND o.release_at - interval '24 hours' > o.delivered_at))";

pub async fn run(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    let due: Vec<Uuid> = sqlx::query_scalar(&format!(
        "SELECT o.id FROM orders o WHERE {DUE} ORDER BY o.release_at, o.id LIMIT $1"
    ))
    .bind(BATCH)
    .fetch_all(&state.db)
    .await
    .context("find orders due a reminder")?;
    let mut stats = Stats::default();
    for order_id in due {
        if cancel.is_cancelled() {
            break;
        }
        match remind(state, order_id).await {
            Ok(true) => stats.done += 1,
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(job = CONFIRM_REMINDERS, error = %e, "reminder not raised");
                stats.failed += 1;
            }
        }
    }
    Ok(stats)
}

/// Raises the stage due now for one order. `false` when the order is locked by
/// someone else (the next tick tries again) or is no longer due.
async fn remind(state: &AppState, order_id: Uuid) -> Result<bool, AppError> {
    let mut tx = state.db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    // Re-checked under the lock: a release, a hold or a dispute may have won.
    let row: Option<(Uuid, bool)> = sqlx::query_as(&format!(
        "SELECT o.buyer_id, now() >= o.release_at - interval '24 hours' \
         FROM orders o WHERE o.id = $1 AND {DUE} FOR UPDATE OF o SKIP LOCKED"
    ))
    .bind(order_id)
    .fetch_optional(&mut *tx)
    .await
    .context("lock order")?;
    let Some((buyer, last_day)) = row else {
        return Ok(false);
    };
    let (stage, kind) = if last_day {
        (2_i16, Kind::ReleaseTomorrow)
    } else {
        (1_i16, Kind::ConfirmReminder)
    };
    sqlx::query("UPDATE orders SET confirm_reminders_sent = $2 WHERE id = $1")
        .bind(order_id)
        .bind(stage)
        .execute(&mut *tx)
        .await
        .context("advance reminders")?;
    enqueue(&mut tx, Notice::order(buyer, kind, order_id)).await?;
    tx.commit().await.context("commit")?;
    Ok(true)
}
