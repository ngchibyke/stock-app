//! Retries messages held while the tone check was down.
//!
//! Only the oldest held row per sender and conversation is claimed, so messages
//! arrive in the order they were sent. Claiming pushes `next_attempt_at`
//! forward as a lease, in one statement, so no lock is held during the tone
//! check. [`apply_verdict`] re-checks the status under the row lock, which is
//! what stops two runs delivering twice.
//!
//! A held message is withheld, with no strike, if the check has not recovered
//! after [`GIVE_UP_AFTER`], or if the chat has closed or the sender has been
//! muted or banned since: the sender is told, and nobody is left waiting.

use anyhow::Context;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    send::{apply_verdict, recent_texts},
    service,
};
use crate::{
    AppState,
    error::AppError,
    escrow::jobs::{PROVIDER_CALLS_PER_TICK, Stats},
    limits::MAX_CHAT_CHARS,
    moderation::{
        filter::Category,
        pipeline::{FlagSeverity, Source, Verdict, judge},
        strikes::ensure_can_post,
    },
};

/// Job name (`job_runs.job`).
pub const CHAT_HELD: &str = "chat_held";
/// A held message is withheld this long after it was sent.
pub const GIVE_UP_AFTER: &str = "6 hours";
/// The longest wait between tries, in minutes.
const MAX_BACKOFF_MINUTES: i32 = 15;

#[derive(sqlx::FromRow)]
struct Claimed {
    id: Uuid,
    conversation_id: Uuid,
    sender_id: Uuid,
    body_enc: Option<Vec<u8>>,
    expired: bool,
}

pub async fn run(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    let claimed: Vec<Claimed> = sqlx::query_as(&format!(
        "WITH due AS ( \
           SELECT m.id FROM chat_messages m \
           WHERE m.status = 'held' AND m.next_attempt_at <= now() \
             AND NOT EXISTS ( \
               SELECT 1 FROM chat_messages older \
               WHERE older.conversation_id = m.conversation_id \
                 AND older.sender_id = m.sender_id AND older.status = 'held' \
                 AND (older.created_at, older.id) < (m.created_at, m.id)) \
           ORDER BY m.next_attempt_at, m.id LIMIT $1 FOR UPDATE OF m SKIP LOCKED) \
         UPDATE chat_messages c SET next_attempt_at = now() + interval '2 minutes' \
         FROM due WHERE c.id = due.id \
         RETURNING c.id, c.conversation_id, c.sender_id, c.body_enc, \
                   c.created_at < now() - interval '{GIVE_UP_AFTER}' AS expired"
    ))
    .bind(PROVIDER_CALLS_PER_TICK)
    .fetch_all(&state.db)
    .await
    .context("claim held messages")?;

    let mut stats = Stats::default();
    for row in claimed {
        if cancel.is_cancelled() {
            break;
        }
        match settle(state, &row).await {
            Ok(true) => stats.done += 1,
            Ok(false) => back_off(state, row.id).await?,
            Err(e) => {
                tracing::warn!(job = CHAT_HELD, error = %e, "held message not processed");
                stats.failed += 1;
                back_off(state, row.id).await?;
            }
        }
    }
    Ok(stats)
}

/// Gives the row its verdict. False when the tone check is still down.
async fn settle(state: &AppState, row: &Claimed) -> Result<bool, AppError> {
    let access = service::access(&state.db, row.sender_id, row.conversation_id).await?;
    let mut conn = state.db.acquire().await.context("acquire")?;
    let barred = match ensure_can_post(&mut conn, row.sender_id).await {
        Ok(()) => false,
        Err(AppError::Muted(_) | AppError::Forbidden(_)) => true,
        Err(e) => return Err(e),
    };
    if row.expired || barred || !access.writable {
        drop(conn);
        apply_verdict(state, row.id, &give_up()).await?;
        return Ok(true);
    }
    let Some(encrypted) = &row.body_enc else {
        return Err(anyhow::anyhow!("a held message has no body").into());
    };
    let text = state.cipher.decrypt(encrypted)?;
    let recent = recent_texts(&mut conn, &state.cipher, row.conversation_id, row.sender_id).await?;
    drop(conn);
    let recent: Vec<&str> = recent.iter().map(String::as_str).collect();
    match judge(state.tone.as_ref(), &text, &recent, MAX_CHAT_CHARS).await {
        Ok(verdict) => {
            apply_verdict(state, row.id, &verdict).await?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

fn give_up() -> Verdict {
    Verdict::Block {
        severity: FlagSeverity::Unreviewable,
        category: Category::Unreviewable,
        source: Source::System,
    }
}

/// `min(2^attempts, 15)` minutes, so a failing row is not retried every tick
/// and cannot fill the batch.
async fn back_off(state: &AppState, id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE chat_messages SET attempts = attempts + 1, \
                next_attempt_at = now() + make_interval(mins => \
                    LEAST(power(2, LEAST(attempts + 1, 10))::int, $2)) \
         WHERE id = $1 AND status = 'held'",
    )
    .bind(id)
    .bind(MAX_BACKOFF_MINUTES)
    .execute(&state.db)
    .await
    .context("back off held message")?;
    Ok(())
}
