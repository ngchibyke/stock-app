//! The push loop: sends the notification outbox to devices.
//!
//! **Claim, then send.** [`run_once`] claims up to [`CLAIM`] due notices in a
//! short transaction (`FOR UPDATE SKIP LOCKED`), marking each `sending` with a
//! [`LEASE`]-long `push_until`, and commits. The sends happen after that, with
//! no transaction open, at most [`IN_FLIGHT`] at a time. A notice whose worker
//! died is claimed again once its lease runs out.
//!
//! **Lease renewed at the send.** A claimed notice may wait behind others for
//! a permit. Its lease is taken afresh when its turn comes, so a second
//! worker cannot take a notice that is being sent and send it again.
//!
//! **Fenced outcome.** The outcome is written only if the notice still has
//! the `push_until` this worker set: an answer that arrives after the lease
//! passed to someone else changes nothing.
//!
//! **Who gets a push.** Devices whose session is neither revoked nor expired,
//! that are not disabled, and whose user is not deleted. Devices that already
//! got this notice (`pushed_devices`) are never sent it again.
//!
//! **Outcomes.**
//! - every device answered, or none is left to try: `sent`;
//! - no eligible device, and none was ever sent to: `skipped`;
//! - older than [`STALE_HOURS`] when claimed: `skipped`, not sent;
//! - any device unavailable: back to `pending` after a backoff of 30 seconds,
//!   doubling, capped at an hour; after [`MAX_ATTEMPTS`] attempts, `failed`;
//! - a dead token: that device is disabled (`unregistered`);
//! - a refusal: that device is treated as done.
//!
//! The inbox row is the notice itself, so it exists whatever push does.

use std::{sync::Arc, time::Duration};

use anyhow::Context;
use serde_json::json;
use time::OffsetDateTime;
use tokio::{sync::Semaphore, task::JoinSet};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    kinds::Kind,
    push::{PushError, PushMessage},
};
use crate::{
    AppState, audit,
    error::AppError,
    escrow::jobs::{Stats, run_every},
};

/// Notices claimed per tick.
pub const CLAIM: i64 = 100;
/// Push calls in flight at once.
pub const IN_FLIGHT: usize = 8;
/// A notice this old when claimed is skipped: it is no longer news.
pub const STALE_HOURS: i32 = 24;
/// Attempts before a notice is given up as `failed`.
pub const MAX_ATTEMPTS: i32 = 6;
/// How often the loop runs.
pub const PERIOD: Duration = Duration::from_secs(5);
/// How long a claim lasts (`push_until`).
const LEASE: Duration = Duration::from_secs(120);

#[derive(sqlx::FromRow)]
struct Claimed {
    id: Uuid,
    user_id: Uuid,
    kind: String,
    stale: bool,
    push_attempts: i32,
    pushed_devices: Vec<Uuid>,
    push_until: OffsetDateTime,
}

/// What a notice's sends came to.
enum Outcome {
    Sent,
    Skipped,
    /// Some device was unavailable.
    Retry,
}

/// One pass over the outbox (see the module docs). `done` counts notices
/// settled (`sent` or `skipped`), `failed` those put back or given up.
pub async fn run_once(state: &AppState) -> Result<Stats, AppError> {
    run_once_with_lease(state, LEASE).await
}

/// [`run_once`] with a custom lease (tests).
pub async fn run_once_with_lease(state: &AppState, lease: Duration) -> Result<Stats, AppError> {
    let claimed: Vec<Claimed> = sqlx::query_as(
        "UPDATE notifications n SET push_status = 'sending', push_until = now() + make_interval(secs => $3) \
         WHERE n.id IN ( \
             SELECT id FROM notifications \
             WHERE (push_status = 'pending' AND next_push_at <= now()) \
                OR (push_status = 'sending' AND push_until <= now()) \
             ORDER BY next_push_at, id LIMIT $1 FOR UPDATE SKIP LOCKED) \
         RETURNING n.id, n.user_id, n.kind, \
                   n.created_at < now() - make_interval(hours => $2) AS stale, \
                   n.push_attempts, n.pushed_devices, n.push_until",
    )
    .bind(CLAIM)
    .bind(STALE_HOURS)
    .bind(lease.as_secs_f64())
    .fetch_all(&state.db)
    .await
    .context("claim notices")?;

    let limit = Arc::new(Semaphore::new(IN_FLIGHT));
    let mut tasks = JoinSet::new();
    for notice in claimed {
        let (state, limit) = (state.clone(), limit.clone());
        tasks.spawn(async move { push_one(&state, &limit, lease, notice).await });
    }
    let mut stats = Stats::default();
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok(true)) => stats.done += 1,
            Ok(Ok(false)) => stats.failed += 1,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "push outcome not recorded");
                stats.failed += 1;
            }
            Err(_) => {
                tracing::error!("push task panicked");
                stats.failed += 1;
            }
        }
    }
    Ok(stats)
}

/// The production loop: [`run_once`] every [`PERIOD`] until `cancel` fires.
pub async fn run_loop(state: AppState, cancel: CancellationToken) {
    run_every(PERIOD, cancel, move || {
        let state = state.clone();
        async move {
            match run_once(&state).await {
                Ok(s) if s.done + s.failed > 0 => {
                    tracing::info!(settled = s.done, unsettled = s.failed, "push tick");
                }
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "push tick failed"),
            }
        }
    })
    .await
}

/// Sends one claimed notice and records the outcome. `true` when it settled.
async fn push_one(
    state: &AppState,
    limit: &Semaphore,
    lease: Duration,
    mut n: Claimed,
) -> Result<bool, AppError> {
    // One notice at a time per permit, its devices in turn: at most
    // `IN_FLIGHT` calls are ever out.
    let _permit = limit.acquire().await.context("push limit closed")?;
    // The claim was taken when the batch was, and this notice may have waited
    // behind others since. Take a fresh lease now, so it lasts for the send;
    // if another worker took the notice meanwhile, it is theirs.
    let renewed: Option<OffsetDateTime> = sqlx::query_scalar(
        "UPDATE notifications SET push_until = now() + make_interval(secs => $3) \
         WHERE id = $1 AND push_status = 'sending' AND push_until = $2 \
         RETURNING push_until",
    )
    .bind(n.id)
    .bind(n.push_until)
    .bind(lease.as_secs_f64())
    .fetch_optional(&state.db)
    .await
    .context("renew push lease")?;
    let Some(push_until) = renewed else {
        return Ok(false);
    };
    n.push_until = push_until;
    let mut pushed = n.pushed_devices.clone();
    let outcome = match Kind::parse(&n.kind) {
        _ if n.stale => Outcome::Skipped,
        // A kind this build does not know: nothing it could say.
        None => Outcome::Skipped,
        Some(kind) => send_to_devices(state, &n, kind, &mut pushed).await?,
    };
    let fence = "id = $1 AND push_status = 'sending' AND push_until = $2";
    let written = match outcome {
        Outcome::Sent | Outcome::Skipped => {
            let status = if matches!(outcome, Outcome::Sent) {
                "sent"
            } else {
                "skipped"
            };
            sqlx::query(&format!(
                "UPDATE notifications SET push_status = $3, push_until = NULL, \
                        pushed_devices = $4 WHERE {fence}"
            ))
            .bind(n.id)
            .bind(n.push_until)
            .bind(status)
            .bind(&pushed)
            .execute(&state.db)
            .await
        }
        Outcome::Retry => {
            // 30s, doubling per attempt already made, capped at an hour.
            sqlx::query(&format!(
                "UPDATE notifications SET push_attempts = push_attempts + 1, \
                        push_until = NULL, pushed_devices = $3, \
                        push_status = CASE WHEN push_attempts + 1 >= $4 THEN 'failed' \
                                           ELSE 'pending' END, \
                        next_push_at = now() + LEAST( \
                            make_interval(secs => 30 * power(2, LEAST(push_attempts, 20))), \
                            interval '1 hour') \
                 WHERE {fence}"
            ))
            .bind(n.id)
            .bind(n.push_until)
            .bind(&pushed)
            .bind(MAX_ATTEMPTS)
            .execute(&state.db)
            .await
        }
    }
    .context("record push outcome")?;
    if written.rows_affected() == 0 {
        tracing::warn!("push answer arrived after its lease; not recorded");
        return Ok(false);
    }
    if matches!(outcome, Outcome::Retry) && n.push_attempts + 1 >= MAX_ATTEMPTS {
        tracing::warn!(attempts = MAX_ATTEMPTS, "push given up");
    }
    Ok(!matches!(outcome, Outcome::Retry))
}

/// Sends to every eligible device not yet in `pushed`, adding each device that
/// is done (sent, or refused for good) to it.
async fn send_to_devices(
    state: &AppState,
    n: &Claimed,
    kind: Kind,
    pushed: &mut Vec<Uuid>,
) -> Result<Outcome, AppError> {
    let devices: Vec<(Uuid, Vec<u8>)> = sqlx::query_as(
        "SELECT d.id, d.token_enc FROM push_devices d \
         JOIN sessions s ON s.id = d.session_id \
         JOIN users u ON u.id = d.user_id \
         WHERE d.user_id = $1 AND d.disabled_at IS NULL \
           AND s.revoked_at IS NULL AND s.expires_at > now() \
           AND u.deleted_at IS NULL AND d.id <> ALL($2) \
         ORDER BY d.id",
    )
    .bind(n.user_id)
    .bind(&*pushed)
    .fetch_all(&state.db)
    .await
    .context("load push devices")?;
    if devices.is_empty() {
        return Ok(if pushed.is_empty() {
            Outcome::Skipped
        } else {
            Outcome::Sent
        });
    }
    let mut retry = false;
    for (device_id, token_enc) in devices {
        let Ok(token) = state.cipher.decrypt(&token_enc) else {
            // Unreadable for good: treat the device as done.
            tracing::error!("push token could not be decrypted");
            pushed.push(device_id);
            continue;
        };
        let message = PushMessage {
            token: &token,
            title: kind.title(),
            body: kind.body(),
            kind: kind.as_str(),
            notification_id: n.id,
        };
        match state.push.send(&message).await {
            Ok(()) => pushed.push(device_id),
            Err(PushError::Rejected) => {
                tracing::warn!(error = "rejected", "push refused for a device");
                pushed.push(device_id);
            }
            Err(PushError::Unregistered) => disable_device(state, device_id).await?,
            Err(PushError::Unavailable) => retry = true,
        }
    }
    Ok(if retry { Outcome::Retry } else { Outcome::Sent })
}

/// Retires a device whose token the provider says is dead, with its audit
/// row (the device id only, never the token).
async fn disable_device(state: &AppState, device_id: Uuid) -> Result<(), AppError> {
    let mut tx = state.db.begin().await.context("begin")?;
    let disabled = sqlx::query(
        "UPDATE push_devices SET disabled_at = now(), disabled_reason = 'unregistered' \
         WHERE id = $1 AND disabled_at IS NULL",
    )
    .bind(device_id)
    .execute(&mut *tx)
    .await
    .context("disable push device")?;
    if disabled.rows_affected() == 1 {
        audit::record(
            &mut tx,
            None,
            "push_device.disabled",
            "push_device",
            Some(device_id),
            json!({ "reason": "unregistered" }),
        )
        .await?;
    }
    tx.commit().await.context("commit")?;
    Ok(())
}
