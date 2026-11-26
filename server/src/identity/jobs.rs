//! Background jobs for the identity check.
//!
//! - `identity_attempts` (job 20): asks the provider about attempts that got no webhook
//!   within [`service::POLL_AFTER_MINUTES`], and closes attempts older than
//!   [`service::STALE_MINUTES`] so a seller is never stuck mid-attempt.
//! - `identity_grace` (job 21): stores whose grace period has run out with an unverified
//!   owner go `pending` (the plot is kept), and owners inside the period get one
//!   reminder a week.

use anyhow::Context;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::service::{self, STALE_MINUTES};
use crate::{
    AppState, audit,
    error::AppError,
    escrow::jobs::Stats,
    notify::{
        kinds::Kind,
        service::{Notice, enqueue},
    },
};

pub const IDENTITY_ATTEMPTS: &str = "identity_attempts";
pub const IDENTITY_GRACE: &str = "identity_grace";
const BATCH: i64 = 50;

/// Job 20.
pub async fn run_attempts(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    let mut stats = Stats::default();
    // Poll first: a result found now beats closing the attempt as abandoned.
    if let Some(provider) = state.identity.as_deref() {
        let due: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, provider_ref FROM identity_checks \
             WHERE status = 'started' AND provider = $1 AND provider_ref IS NOT NULL \
               AND started_at < now() - make_interval(mins => $2) \
             ORDER BY started_at LIMIT $3",
        )
        .bind(provider.name())
        .bind(service::POLL_AFTER_MINUTES)
        .bind(BATCH)
        .fetch_all(&state.db)
        .await
        .context("find attempts to poll")?;
        for (id, provider_ref) in due {
            if cancel.is_cancelled() {
                break;
            }
            match provider.fetch(&provider_ref).await {
                Ok(Some(outcome)) => match service::apply_outcome(state, id, &outcome).await {
                    Ok(service::Applied::Done) => stats.done += 1,
                    Ok(service::Applied::Ignored) => {}
                    Err(_) => stats.failed += 1,
                },
                Ok(None) => {}
                Err(_) => stats.failed += 1,
            }
        }
    }
    let expired: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE identity_checks SET status = 'failed', reason = 'expired', decided_at = now() \
         WHERE id IN (SELECT id FROM identity_checks WHERE status = 'started' \
                      AND started_at < now() - make_interval(mins => $1) \
                      ORDER BY started_at LIMIT $2) \
           AND status = 'started' RETURNING id",
    )
    .bind(STALE_MINUTES)
    .bind(BATCH)
    .fetch_all(&state.db)
    .await
    .context("expire attempts")?;
    stats.done += u32::try_from(expired.len()).unwrap_or(u32::MAX);
    Ok(stats)
}

/// Job 21.
pub async fn run_grace(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    let mut stats = Stats::default();
    let due: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT s.id, s.owner_id FROM stores s \
         WHERE s.status = 'active' AND s.deleted_at IS NULL \
           AND s.identity_due_at IS NOT NULL AND s.identity_due_at <= now() \
           AND NOT EXISTS (SELECT 1 FROM identity_checks ic \
                           WHERE ic.user_id = s.owner_id AND ic.status = 'passed') \
         ORDER BY s.identity_due_at LIMIT $1",
    )
    .bind(BATCH)
    .fetch_all(&state.db)
    .await
    .context("find stores past grace")?;
    for (store, owner) in due {
        if cancel.is_cancelled() {
            break;
        }
        let mut tx = state.db.begin().await.context("begin")?;
        // The status flips only if nothing changed since the read.
        let changed = sqlx::query(
            "UPDATE stores SET status = 'pending' WHERE id = $1 AND status = 'active' \
               AND identity_due_at <= now() \
               AND NOT EXISTS (SELECT 1 FROM identity_checks ic \
                               WHERE ic.user_id = stores.owner_id AND ic.status = 'passed')",
        )
        .bind(store)
        .execute(&mut *tx)
        .await
        .context("park store")?
        .rows_affected();
        if changed == 1 {
            audit::record(
                &mut tx,
                None,
                "identity.grace_expired",
                "store",
                Some(store),
                serde_json::json!({}),
            )
            .await?;
            enqueue(
                &mut tx,
                Notice::user(
                    owner,
                    Kind::IdentityDue,
                    format!("identity_due:expired:{store}"),
                ),
            )
            .await?;
            stats.done += 1;
        }
        tx.commit().await.context("commit")?;
    }
    // One reminder a week while inside the period.
    let remind: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT s.owner_id FROM stores s \
         WHERE s.status = 'active' AND s.deleted_at IS NULL AND s.identity_due_at > now() \
           AND NOT EXISTS (SELECT 1 FROM identity_checks ic \
                           WHERE ic.user_id = s.owner_id AND ic.status = 'passed') \
         LIMIT $1",
    )
    .bind(BATCH)
    .fetch_all(&state.db)
    .await
    .context("find owners to remind")?;
    for owner in remind {
        if cancel.is_cancelled() {
            break;
        }
        let mut conn = state.db.acquire().await.context("acquire")?;
        let week: String =
            sqlx::query_scalar("SELECT to_char(now() AT TIME ZONE 'Africa/Lagos', 'IYYY-IW')")
                .fetch_one(&mut *conn)
                .await
                .context("read week")?;
        enqueue(
            &mut conn,
            Notice::user(owner, Kind::IdentityDue, format!("identity_due:{week}")),
        )
        .await?;
    }
    Ok(stats)
}
