//! The identity check's rules: starting an attempt, applying the provider's result,
//! store activation, bans, the owner's decisions and revokes.
//!
//! Row locks: users, then the user's stores, then `identity_checks`.
//! Audit rows hold ids, statuses and reason codes only; never a NIN, a name or a note.

use std::time::Duration;

use anyhow::Context;
use serde::Serialize;
use serde_json::json;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{Nin, Outcome, fingerprint, names_match};
use crate::{
    AppState, audit,
    auth::AuthUser,
    checkout::service::busy_err,
    error::AppError,
    market::layout::Layout,
    notify::{
        kinds::Kind,
        service::{Notice, enqueue},
    },
    payout_accounts::service::sync_store_status,
    stores::access::require_seller_or_member,
};

/// An attempt with no result after this long is abandoned.
pub const STALE_MINUTES: i32 = 30;
/// The poll job asks the provider about attempts older than this with no webhook yet.
pub const POLL_AFTER_MINUTES: i32 = 5;
const LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";
const NOT_ACCEPTED: &str = "We could not verify this NIN. Contact support.";
const BUSY: &str = "identity_busy";

async fn begin(db: &PgPool) -> Result<Transaction<'static, Postgres>, AppError> {
    let mut tx = db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    Ok(tx)
}

fn not_accepted() -> AppError {
    AppError::ConflictDetail("identity_not_accepted", NOT_ACCEPTED.into())
}

#[derive(sqlx::FromRow)]
struct Limits {
    per_day: i32,
    total: i32,
    budget: i32,
    delay_ms: i32,
}

async fn limits(conn: &mut PgConnection) -> Result<Limits, AppError> {
    Ok(sqlx::query_as(
        "SELECT identity_attempts_per_day AS per_day, identity_attempts_total AS total, \
                identity_daily_budget AS budget, identity_refusal_delay_ms AS delay_ms \
         FROM platform_settings WHERE id",
    )
    .fetch_one(conn)
    .await
    .context("load identity settings")?)
}

async fn exists(conn: &mut PgConnection, sql: &str, user: Uuid) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(sql)
        .bind(user)
        .fetch_one(conn)
        .await
        .context("identity lookup")?)
}

/// Locks the user row, then the user's stores in id order.
async fn lock_user_and_stores(conn: &mut PgConnection, user: Uuid) -> Result<(), AppError> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
            .bind(user)
            .fetch_optional(&mut *conn)
            .await
            .map_err(busy_err(BUSY, "lock user"))?;
    found.ok_or(AppError::NotFound)?;
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM stores WHERE owner_id = $1 ORDER BY id FOR NO KEY UPDATE",
    )
    .bind(user)
    .fetch_all(&mut *conn)
    .await
    .map_err(busy_err(BUSY, "lock stores"))?;
    Ok(())
}

async fn insert_row(
    conn: &mut PgConnection,
    user: Uuid,
    provider: &str,
    status: &str,
    reason: Option<&str>,
    nin_idx: Option<&[u8]>,
    called_provider: bool,
) -> Result<Uuid, AppError> {
    Ok(sqlx::query_scalar(
        "INSERT INTO identity_checks \
           (user_id, provider, status, reason, nin_idx, called_provider, decided_at) \
         VALUES ($1, $2, $3, $4, $5, $6, CASE WHEN $3 = 'started' THEN NULL ELSE now() END) \
         RETURNING id",
    )
    .bind(user)
    .bind(provider)
    .bind(status)
    .bind(reason)
    .bind(nin_idx)
    .bind(called_provider)
    .fetch_one(conn)
    .await
    .context("insert identity check")?)
}

#[derive(Debug, Serialize)]
pub struct StartView {
    pub attempt_id: Uuid,
    pub provider: &'static str,
    pub session_token: String,
    pub expires_in_secs: i64,
}

/// Why a start did not reach the provider.
enum Stop {
    Refused,
    InReview,
    RateLimited,
    Budget,
}

/// `POST /me/identity/start`: validates, applies the caps and the ban and duplicate
/// checks on our side, then asks the provider for a job. The NIN lives in `nin` for
/// this call only.
pub async fn start(
    state: &AppState,
    user: &AuthUser,
    raw_nin: &str,
) -> Result<StartView, AppError> {
    let nin = Nin::parse(raw_nin)
        .ok_or_else(|| AppError::Validation("nin must be exactly 11 digits".into()))?;
    let (Some(provider), Some(key)) = (state.identity.as_deref(), state.nin_key.as_deref()) else {
        return Err(AppError::Unavailable("identity_unavailable"));
    };
    {
        let mut conn = state.db.acquire().await.context("acquire")?;
        require_seller_or_member(&mut conn, user).await?;
    }
    let idx = fingerprint(key, &nin);
    let mut tx = begin(&state.db).await?;
    let lim = limits(&mut tx).await?;
    let user_id = user.id;
    lock_user_and_stores(&mut tx, user_id).await?;

    if exists(
        &mut tx,
        "SELECT EXISTS (SELECT 1 FROM identity_checks WHERE user_id = $1 AND status = 'passed')",
        user_id,
    )
    .await?
    {
        return Err(AppError::Conflict("identity_already_verified"));
    }
    if exists(
        &mut tx,
        "SELECT EXISTS (SELECT 1 FROM identity_checks WHERE user_id = $1 AND status = 'review')",
        user_id,
    )
    .await?
    {
        return Err(AppError::Conflict("identity_in_review"));
    }
    // One live attempt at a time; an abandoned one is closed first.
    let open: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT id, started_at < now() - make_interval(mins => $2) \
         FROM identity_checks WHERE user_id = $1 AND status = 'started'",
    )
    .bind(user_id)
    .bind(STALE_MINUTES)
    .fetch_optional(&mut *tx)
    .await
    .context("read open attempt")?;
    if let Some((id, stale)) = open {
        if !stale {
            return Err(AppError::Conflict("identity_in_progress"));
        }
        sqlx::query(
            "UPDATE identity_checks SET status = 'failed', reason = 'expired', decided_at = now() \
             WHERE id = $1 AND status = 'started'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .context("expire attempt")?;
    }

    // Caps. A provider outage and an unverified review row are not attempts.
    let (today, total): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE started_at > now() - interval '24 hours'), count(*) \
         FROM identity_checks WHERE user_id = $1 \
           AND reason IS DISTINCT FROM 'attempts_exhausted' \
           AND reason IS DISTINCT FROM 'provider_error'",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await
    .context("count attempts")?;

    let stop = if today >= i64::from(lim.per_day) {
        Some(Stop::RateLimited)
    } else if total >= i64::from(lim.total) {
        insert_row(
            &mut tx,
            user_id,
            provider.name(),
            "review",
            Some("attempts_exhausted"),
            Some(&idx),
            false,
        )
        .await?;
        audit::record(
            &mut tx,
            Some(user_id),
            "identity.review",
            "user",
            Some(user_id),
            json!({ "reason": "attempts_exhausted" }),
        )
        .await?;
        Some(Stop::InReview)
    } else if let Some(why) = refusal(&mut tx, user_id, &idx).await? {
        insert_row(
            &mut tx,
            user_id,
            provider.name(),
            "failed",
            Some("not_accepted"),
            None,
            false,
        )
        .await?;
        audit::record(
            &mut tx,
            Some(user_id),
            "identity.refused",
            "user",
            Some(user_id),
            json!({ "why": why }),
        )
        .await?;
        Some(Stop::Refused)
    } else {
        let spent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM identity_checks \
             WHERE called_provider AND started_at > now() - interval '24 hours'",
        )
        .fetch_one(&mut *tx)
        .await
        .context("count provider calls")?;
        if spent >= i64::from(lim.budget) {
            let alerted: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM audit_log WHERE action = 'identity.budget_exhausted' \
                 AND at > now() - interval '1 hour')",
            )
            .fetch_one(&mut *tx)
            .await
            .context("read budget alert")?;
            if !alerted {
                audit::record(
                    &mut tx,
                    None,
                    "identity.budget_exhausted",
                    "platform",
                    None,
                    json!({}),
                )
                .await?;
            }
            Some(Stop::Budget)
        } else {
            None
        }
    };
    if let Some(stop) = stop {
        tx.commit().await.context("commit")?;
        return Err(match stop {
            Stop::Refused => {
                // A refusal takes as long as a provider round trip, so timing
                // does not say why.
                tokio::time::sleep(Duration::from_millis(lim.delay_ms as u64)).await;
                not_accepted()
            }
            Stop::InReview => AppError::Conflict("identity_in_review"),
            Stop::RateLimited => AppError::TooManyRequests("identity_rate_limited"),
            Stop::Budget => AppError::Unavailable("identity_unavailable"),
        });
    }

    let id = insert_row(
        &mut tx,
        user_id,
        provider.name(),
        "started",
        None,
        Some(&idx),
        true,
    )
    .await?;
    tx.commit().await.context("commit")?;

    // The provider call happens outside any transaction.
    match provider.start(&nin, id).await {
        Ok(job) => {
            let mut conn = state.db.acquire().await.context("acquire")?;
            sqlx::query(
                "UPDATE identity_checks SET provider_ref = $2 WHERE id = $1 AND status = 'started'",
            )
            .bind(id)
            .bind(&job.provider_ref)
            .execute(&mut *conn)
            .await
            .context("save provider reference")?;
            audit::record(
                &mut conn,
                Some(user_id),
                "identity.started",
                "identity_check",
                Some(id),
                json!({ "provider": provider.name() }),
            )
            .await?;
            Ok(StartView {
                attempt_id: id,
                provider: provider.name(),
                session_token: job.session_token,
                expires_in_secs: i64::from(STALE_MINUTES) * 60,
            })
        }
        Err(_) => {
            // Never stuck mid-attempt, and an outage is not the seller's attempt.
            sqlx::query(
                "UPDATE identity_checks SET status = 'failed', reason = 'provider_error', \
                        called_provider = false, decided_at = now() \
                 WHERE id = $1 AND status = 'started'",
            )
            .bind(id)
            .execute(&state.db)
            .await
            .context("close attempt")?;
            tracing::warn!(provider = provider.name(), "identity provider start failed");
            Err(AppError::Unavailable("identity_unavailable"))
        }
    }
}

/// The audit-only reason a start is refused: a banned user, a revoked check, a banned
/// NIN, or a NIN another account holds. The caller answers all of them the same way.
async fn refusal(
    conn: &mut PgConnection,
    user: Uuid,
    idx: &[u8],
) -> Result<Option<&'static str>, AppError> {
    if exists(
        conn,
        "SELECT EXISTS (SELECT 1 FROM user_moderation WHERE user_id = $1 AND banned_at IS NOT NULL)",
        user,
    )
    .await?
    {
        return Ok(Some("banned_user"));
    }
    if exists(
        conn,
        "SELECT EXISTS (SELECT 1 FROM identity_checks WHERE user_id = $1 AND status = 'revoked')",
        user,
    )
    .await?
    {
        return Ok(Some("revoked_user"));
    }
    if nin_banned(conn, idx).await? {
        return Ok(Some("banned_nin"));
    }
    if nin_held_by_other(conn, idx, user).await? {
        return Ok(Some("nin_in_use"));
    }
    Ok(None)
}

async fn nin_banned(conn: &mut PgConnection, idx: &[u8]) -> Result<bool, AppError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM banned_nins WHERE nin_idx = $1)")
            .bind(idx)
            .fetch_one(conn)
            .await
            .context("read banned nins")?,
    )
}

async fn nin_held_by_other(
    conn: &mut PgConnection,
    idx: &[u8],
    user: Uuid,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_checks WHERE nin_idx = $1 AND user_id <> $2 \
           AND (status IN ('passed', 'revoked') \
                OR (status = 'review' AND reason IS DISTINCT FROM 'attempts_exhausted')))",
    )
    .bind(idx)
    .bind(user)
    .fetch_one(conn)
    .await
    .context("read nin holders")?)
}

/// What a webhook or poll found for the attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    Done,
    /// Unknown reference, or the attempt was already decided: nothing changed.
    Ignored,
}

/// Applies the provider's answer to the attempt with `provider_ref`. A duplicate or
/// replayed message changes nothing.
pub async fn apply_by_ref(
    state: &AppState,
    provider: &str,
    provider_ref: &str,
    outcome: &Outcome,
) -> Result<Applied, AppError> {
    let check: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM identity_checks WHERE provider = $1 AND provider_ref = $2",
    )
    .bind(provider)
    .bind(provider_ref)
    .fetch_optional(&state.db)
    .await
    .context("find attempt")?;
    match check {
        Some(id) => apply_outcome(state, id, outcome).await,
        None => Ok(Applied::Ignored),
    }
}

pub async fn apply_outcome(
    state: &AppState,
    check_id: Uuid,
    outcome: &Outcome,
) -> Result<Applied, AppError> {
    let user: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM identity_checks WHERE id = $1")
            .bind(check_id)
            .fetch_optional(&state.db)
            .await
            .context("find attempt owner")?;
    let Some(user) = user else {
        return Ok(Applied::Ignored);
    };
    let mut tx = begin(&state.db).await?;
    lock_user_and_stores(&mut tx, user).await?;
    let row: Option<(String, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT status, nin_idx FROM identity_checks WHERE id = $1 FOR UPDATE")
            .bind(check_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(busy_err(BUSY, "lock attempt"))?;
    let Some((status, idx)) = row else {
        return Ok(Applied::Ignored);
    };
    if status != "started" {
        return Ok(Applied::Ignored);
    }

    // (status, reason, verified name, birth year)
    let mut new_status = "failed";
    let mut reason: Option<&str> = None;
    let mut name_enc: Option<Vec<u8>> = None;
    let mut birth_year: Option<i32> = None;
    match outcome {
        Outcome::Failed(r) => reason = Some(r.code()),
        Outcome::Unsure => {
            new_status = "review";
            reason = Some("provider_unsure");
        }
        Outcome::Passed(v) => {
            let blocked = match &idx {
                Some(idx) => {
                    nin_banned(&mut tx, idx).await? || nin_held_by_other(&mut tx, idx, user).await?
                }
                None => false,
            } || exists(
                &mut tx,
                "SELECT EXISTS (SELECT 1 FROM user_moderation WHERE user_id = $1 AND banned_at IS NOT NULL)",
                user,
            )
            .await?;
            if blocked {
                reason = Some("not_accepted");
            } else {
                name_enc = Some(state.cipher.encrypt(&v.full_name));
                birth_year = Some(v.date_of_birth.year());
                if super::age_on(v.date_of_birth, super::today()) < 18 {
                    new_status = "review";
                    reason = Some("under_18");
                } else if !payout_names_ok(&mut tx, state, user, &v.full_name).await? {
                    new_status = "review";
                    reason = Some("name_mismatch");
                } else {
                    new_status = "passed";
                }
            }
        }
    }
    sqlx::query(
        "UPDATE identity_checks SET status = $2, reason = $3, full_name_enc = $4, birth_year = $5, \
                decided_at = CASE WHEN $2 = 'review' THEN NULL ELSE now() END \
         WHERE id = $1",
    )
    .bind(check_id)
    .bind(new_status)
    .bind(reason)
    .bind(name_enc)
    .bind(birth_year)
    .execute(&mut *tx)
    .await
    .context("save identity result")?;
    audit::record(
        &mut tx,
        None,
        match new_status {
            "passed" => "identity.passed",
            "review" => "identity.review",
            _ => "identity.failed",
        },
        "identity_check",
        Some(check_id),
        json!({ "user_id": user, "reason": reason }),
    )
    .await?;
    match new_status {
        "passed" => {
            resync_owner_stores(&mut tx, &state.market, user).await?;
            enqueue(
                &mut tx,
                Notice::user(
                    user,
                    Kind::IdentityPassed,
                    format!("identity_passed:{check_id}"),
                ),
            )
            .await?;
        }
        "failed" => {
            enqueue(
                &mut tx,
                Notice::user(
                    user,
                    Kind::IdentityDeclined,
                    format!("identity_declined:{check_id}"),
                ),
            )
            .await?;
        }
        _ => {}
    }
    tx.commit().await.context("commit")?;
    Ok(Applied::Done)
}

/// True when every verified payout account of the user's stores carries a name that is
/// the same person's as `identity_name`. Read in memory, nothing is written.
async fn payout_names_ok(
    conn: &mut PgConnection,
    state: &AppState,
    owner: Uuid,
    identity_name: &str,
) -> Result<bool, AppError> {
    let names: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT pa.account_name_enc FROM payout_accounts pa JOIN stores s ON s.id = pa.store_id \
         WHERE s.owner_id = $1 AND s.deleted_at IS NULL AND pa.status = 'verified' \
           AND pa.account_name_enc IS NOT NULL",
    )
    .bind(owner)
    .fetch_all(conn)
    .await
    .context("read payout names")?;
    for blob in names {
        let name = state.cipher.decrypt(&blob)?;
        if !names_match(identity_name, &name) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Re-runs the store status rule for each of the owner's live stores (activate those
/// that now qualify, park those that no longer do). The stores must be locked.
pub async fn resync_owner_stores(
    tx: &mut Transaction<'_, Postgres>,
    layout: &Layout,
    owner: Uuid,
) -> Result<(), AppError> {
    let stores: Vec<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT s.id, s.status::text, \
                EXISTS (SELECT 1 FROM payout_accounts pa \
                        WHERE pa.store_id = s.id AND pa.status = 'verified') \
         FROM stores s WHERE s.owner_id = $1 AND s.deleted_at IS NULL ORDER BY s.id",
    )
    .bind(owner)
    .fetch_all(&mut **tx)
    .await
    .context("read owner stores")?;
    for (id, status, payout_verified) in stores {
        sync_store_status(tx, layout, id, &status, payout_verified).await?;
    }
    Ok(())
}

/// True when the store's owner has passed the check, or the store is still inside its
/// grace period. Stores that go active after the check shipped have no grace.
pub async fn store_clear(conn: &mut PgConnection, store_id: Uuid) -> Result<bool, AppError> {
    let clear: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity_checks ic \
                        WHERE ic.user_id = s.owner_id AND ic.status = 'passed') \
                OR (s.identity_due_at IS NOT NULL AND s.identity_due_at > now()) \
         FROM stores s WHERE s.id = $1",
    )
    .bind(store_id)
    .fetch_optional(conn)
    .await
    .context("read store identity")?;
    Ok(clear.unwrap_or(false))
}

/// A manager or staff member may act on orders and edit stock only once they have
/// passed. Owners are not asked here (their stores are held back instead).
pub async fn member_cleared(conn: &mut PgConnection, user: Uuid) -> Result<bool, AppError> {
    exists(
        conn,
        "SELECT EXISTS (SELECT 1 FROM identity_checks WHERE user_id = $1 AND status = 'passed')",
        user,
    )
    .await
}

/// A payout account was verified with the provider-resolved `payout_name`. If the
/// owner's passed identity does not match it, the check goes back to review and the
/// owner's stores wait (the store is already locked by the payout transaction).
pub async fn recheck_payout_name(
    tx: &mut Transaction<'_, Postgres>,
    state: &AppState,
    owner: Uuid,
    payout_name: &str,
) -> Result<(), AppError> {
    let row: Option<(Uuid, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT id, full_name_enc FROM identity_checks \
         WHERE user_id = $1 AND status = 'passed' FOR UPDATE",
    )
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await
    .map_err(busy_err(BUSY, "lock passed check"))?;
    let Some((id, Some(blob))) = row else {
        return Ok(());
    };
    let identity_name = state.cipher.decrypt(&blob)?;
    if names_match(&identity_name, payout_name) {
        return Ok(());
    }
    sqlx::query(
        "UPDATE identity_checks SET status = 'review', reason = 'payout_name_mismatch', \
                decided_at = NULL, decided_by = NULL, decision_note = NULL WHERE id = $1",
    )
    .bind(id)
    .execute(&mut **tx)
    .await
    .context("send check to review")?;
    end_grace(tx, owner).await?;
    audit::record(
        tx,
        None,
        "identity.review",
        "identity_check",
        Some(id),
        json!({ "user_id": owner, "reason": "payout_name_mismatch" }),
    )
    .await?;
    Ok(())
}

/// The owner's stores lose any grace period: they need a passed check now.
async fn end_grace(conn: &mut PgConnection, owner: Uuid) -> Result<(), AppError> {
    sqlx::query("UPDATE stores SET identity_due_at = NULL WHERE owner_id = $1")
        .bind(owner)
        .execute(conn)
        .await
        .context("end grace")?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Pass,
    Fail,
}

/// The owner's decision on a check in review. `note` is the owner's required reason,
/// kept on the row and never in the audit log.
pub async fn decide(
    state: &AppState,
    actor: Uuid,
    check_id: Uuid,
    decision: Decision,
    note: &str,
) -> Result<(), AppError> {
    let user: Uuid = sqlx::query_scalar("SELECT user_id FROM identity_checks WHERE id = $1")
        .bind(check_id)
        .fetch_optional(&state.db)
        .await
        .context("find check")?
        .ok_or(AppError::NotFound)?;
    let mut tx = begin(&state.db).await?;
    lock_user_and_stores(&mut tx, user).await?;
    let (status, idx): (String, Option<Vec<u8>>) =
        sqlx::query_as("SELECT status, nin_idx FROM identity_checks WHERE id = $1 FOR UPDATE")
            .bind(check_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(busy_err(BUSY, "lock check"))?
            .ok_or(AppError::NotFound)?;
    if status != "review" {
        return Err(AppError::Conflict("not_in_review"));
    }
    let (new_status, reason) = match decision {
        Decision::Pass => {
            if let Some(idx) = &idx
                && (nin_banned(&mut tx, idx).await?
                    || nin_held_by_other(&mut tx, idx, user).await?)
            {
                return Err(AppError::Conflict("identity_nin_in_use"));
            }
            ("passed", None)
        }
        Decision::Fail => ("failed", Some("owner_declined")),
    };
    sqlx::query(
        "UPDATE identity_checks SET status = $2, reason = $3, decided_at = now(), \
                decided_by = $4, decision_note = $5 WHERE id = $1",
    )
    .bind(check_id)
    .bind(new_status)
    .bind(reason)
    .bind(actor)
    .bind(note)
    .execute(&mut *tx)
    .await
    .context("save decision")?;
    audit::record(
        &mut tx,
        Some(actor),
        if decision == Decision::Pass {
            "identity.review_passed"
        } else {
            "identity.review_failed"
        },
        "identity_check",
        Some(check_id),
        json!({ "user_id": user }),
    )
    .await?;
    if decision == Decision::Pass {
        resync_owner_stores(&mut tx, &state.market, user).await?;
        enqueue(
            &mut tx,
            Notice::user(
                user,
                Kind::IdentityPassed,
                format!("identity_passed:{check_id}"),
            ),
        )
        .await?;
    } else {
        enqueue(
            &mut tx,
            Notice::user(
                user,
                Kind::IdentityDeclined,
                format!("identity_declined:{check_id}"),
            ),
        )
        .await?;
    }
    tx.commit().await.context("commit")?;
    Ok(())
}

/// The owner withdraws a passed check (fraud found afterwards). The user is unverified,
/// their stores wait, and the NIN stays taken.
pub async fn revoke(
    state: &AppState,
    actor: Uuid,
    check_id: Uuid,
    note: &str,
) -> Result<(), AppError> {
    let user: Uuid = sqlx::query_scalar("SELECT user_id FROM identity_checks WHERE id = $1")
        .bind(check_id)
        .fetch_optional(&state.db)
        .await
        .context("find check")?
        .ok_or(AppError::NotFound)?;
    let mut tx = begin(&state.db).await?;
    lock_user_and_stores(&mut tx, user).await?;
    let changed = sqlx::query(
        "UPDATE identity_checks SET status = 'revoked', reason = 'revoked', decided_at = now(), \
                decided_by = $2, decision_note = $3 WHERE id = $1 AND status = 'passed'",
    )
    .bind(check_id)
    .bind(actor)
    .bind(note)
    .execute(&mut *tx)
    .await
    .map_err(busy_err(BUSY, "revoke check"))?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::Conflict("not_passed"));
    }
    end_grace(&mut tx, user).await?;
    resync_owner_stores(&mut tx, &state.market, user).await?;
    audit::record(
        &mut tx,
        Some(actor),
        "identity.revoked",
        "identity_check",
        Some(check_id),
        json!({ "user_id": user }),
    )
    .await?;
    enqueue(
        &mut tx,
        Notice::user(
            user,
            Kind::IdentityDeclined,
            format!("identity_declined:{check_id}"),
        ),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(())
}

/// A ban blocks the fingerprints of the user's attempts that did not fail, and
/// suspends their stores. Runs inside the ban's transaction with the user locked.
pub async fn on_ban(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    user: Uuid,
) -> Result<(), AppError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM stores WHERE owner_id = $1 ORDER BY id FOR NO KEY UPDATE",
    )
    .bind(user)
    .fetch_all(&mut **tx)
    .await
    .map_err(busy_err(BUSY, "lock stores"))?;
    let blocked = sqlx::query(
        "INSERT INTO banned_nins (nin_idx, user_id, banned_by) \
         SELECT DISTINCT nin_idx, user_id, $2 FROM identity_checks \
         WHERE user_id = $1 AND nin_idx IS NOT NULL AND status <> 'failed' \
         ON CONFLICT DO NOTHING",
    )
    .bind(user)
    .bind(actor)
    .execute(&mut **tx)
    .await
    .context("block nins")?
    .rows_affected();
    let suspended = sqlx::query(
        "UPDATE stores SET status = 'suspended', suspended_at = now(), \
                suspended_reason = 'user_banned', suspended_by = $2 \
         WHERE owner_id = $1 AND deleted_at IS NULL AND status <> 'suspended'",
    )
    .bind(user)
    .bind(actor)
    .execute(&mut **tx)
    .await
    .context("suspend stores")?
    .rows_affected();
    audit::record(
        tx,
        Some(actor),
        "identity.ban_effects",
        "user",
        Some(user),
        json!({ "nins_blocked": blocked, "stores_suspended": suspended }),
    )
    .await?;
    Ok(())
}

/// An unban lifts that user's blocks and returns the stores the ban suspended to
/// where the normal rules put them.
pub async fn on_unban(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    layout: &Layout,
    user: Uuid,
) -> Result<(), AppError> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM stores WHERE owner_id = $1 ORDER BY id FOR NO KEY UPDATE",
    )
    .bind(user)
    .fetch_all(&mut **tx)
    .await
    .map_err(busy_err(BUSY, "lock stores"))?;
    let lifted = sqlx::query("DELETE FROM banned_nins WHERE user_id = $1")
        .bind(user)
        .execute(&mut **tx)
        .await
        .context("lift nin blocks")?
        .rows_affected();
    let parked: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE stores SET status = 'pending', suspended_at = NULL, suspended_reason = NULL, \
                suspended_by = NULL \
         WHERE owner_id = $1 AND status = 'suspended' AND suspended_reason = 'user_banned' \
         RETURNING id",
    )
    .bind(user)
    .fetch_all(&mut **tx)
    .await
    .context("unsuspend stores")?;
    for id in &parked {
        let verified: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM payout_accounts WHERE store_id = $1 AND status = 'verified')",
        )
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .context("read payout")?;
        sync_store_status(tx, layout, *id, "pending", verified).await?;
    }
    audit::record(
        tx,
        Some(actor),
        "identity.unban_effects",
        "user",
        Some(user),
        json!({ "nins_unblocked": lifted, "stores_restored": parked.len() }),
    )
    .await?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct StatusView {
    /// `unverified`, `in_progress`, `passed`, `review`, `failed` or `revoked`. A failed
    /// check never says why: a refusal and a provider failure look the same.
    pub status: &'static str,
    /// The user's own verified name, for them to see.
    pub verified_name: Option<String>,
    /// When an unverified owner's store stops taking orders, if it is inside its grace period.
    #[serde(with = "time::serde::rfc3339::option")]
    pub required_by: Option<OffsetDateTime>,
}

/// `GET /me/identity`.
pub async fn status(state: &AppState, user: Uuid) -> Result<StatusView, AppError> {
    let rows: Vec<(String, Option<Vec<u8>>)> = sqlx::query_as(
        "SELECT status, full_name_enc FROM identity_checks WHERE user_id = $1 \
         ORDER BY (status = 'passed') DESC, started_at DESC LIMIT 1",
    )
    .bind(user)
    .fetch_all(&state.db)
    .await
    .context("read identity status")?;
    let (status, name_enc) = match rows.into_iter().next() {
        None => ("unverified", None),
        Some((s, blob)) => (
            match s.as_str() {
                "started" => "in_progress",
                "passed" => "passed",
                "review" => "review",
                "revoked" => "revoked",
                _ => "failed",
            },
            blob,
        ),
    };
    let verified_name = match (status, name_enc) {
        ("passed", Some(blob)) => Some(state.cipher.decrypt(&blob)?),
        _ => None,
    };
    let required_by: Option<OffsetDateTime> = if status == "passed" {
        None
    } else {
        sqlx::query_scalar(
            "SELECT min(identity_due_at) FROM stores \
             WHERE owner_id = $1 AND status = 'active' AND deleted_at IS NULL \
               AND identity_due_at > now()",
        )
        .bind(user)
        .fetch_one(&state.db)
        .await
        .context("read grace")?
    };
    Ok(StatusView {
        status,
        verified_name,
        required_by,
    })
}
