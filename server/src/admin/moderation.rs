//! Moderation desk. Support sees metadata and resolves flags; only the owner
//! reads a flagged original, and every read of one is audited. A ban stops
//! posting only: orders, tracking and money are untouched.
//!
//! Row locks follow the global order: users, then flags; stores, then reviews.
//! The ban reason lives only in the audit log, and no audit row holds message
//! text.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    Owner, Staff, note, parse_id,
    views::{self, Items},
};
use crate::{
    AppState, audit,
    checkout::service::busy_err,
    error::AppError,
    extract::{AppJson, AppQuery},
    moderation::strikes::recalculate_mute,
    orders::views::Cursor,
    reviews::service::{begin, recount},
};

const BAN_REASON_MAX: usize = 500;
const STATUSES: [&str; 3] = ["open", "upheld", "dismissed"];
const SEVERITIES: [&str; 3] = ["mild", "serious", "unreviewable"];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/moderation/flags", get(flags))
        .route("/admin/moderation/flags/{id}", get(flag))
        .route("/admin/moderation/flags/{id}/resolve", post(resolve))
        .route("/admin/moderation/review-queue", get(review_queue))
        .route("/admin/moderation/users/{id}/unmute", post(unmute))
        .route("/admin/moderation/users/{id}/ban", post(ban))
        .route("/admin/moderation/users/{id}/unban", post(unban))
        .route("/admin/reviews/{id}/hide", post(hide))
        .route("/admin/reviews/{id}/unhide", post(unhide))
}

async fn viewed(state: &AppState, actor: Uuid, view: &str, rows: usize) -> Result<(), AppError> {
    let mut conn = state.db.acquire().await.context("acquire")?;
    views::record_view(&mut conn, actor, view, rows).await
}

/// Metadata only: this shape has no text in it, for anyone.
#[derive(Serialize, sqlx::FromRow)]
struct FlagRow {
    id: Uuid,
    user_id: Uuid,
    surface: String,
    severity: String,
    category: String,
    source: String,
    status: String,
    has_strike: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize)]
struct FlagPage {
    items: Vec<FlagRow>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct FlagQuery {
    status: Option<String>,
    severity: Option<String>,
    before: Option<String>,
    limit: Option<i64>,
}

fn one_of(field: &str, value: Option<&str>, allowed: &[&str]) -> Result<(), AppError> {
    match value {
        Some(v) if !allowed.contains(&v) => Err(AppError::Validation(format!(
            "{field} must be one of {}",
            allowed.join(", ")
        ))),
        _ => Ok(()),
    }
}

async fn flags(
    State(state): State<AppState>,
    staff: Staff,
    AppQuery(q): AppQuery<FlagQuery>,
) -> Result<Json<FlagPage>, AppError> {
    one_of("status", q.status.as_deref(), &STATUSES)?;
    one_of("severity", q.severity.as_deref(), &SEVERITIES)?;
    let limit = views::limit(q.limit)?;
    let cursor = q.before.as_deref().map(Cursor::decode).transpose()?;
    let mut items = list_flags(&state.db, &q, cursor.as_ref(), limit).await?;
    let more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    let next_cursor = match (more, items.last()) {
        (true, Some(last)) => Some(Cursor(last.created_at, last.id).encode()?),
        _ => None,
    };
    viewed(&state, staff.0.id, "moderation_flags", items.len()).await?;
    Ok(Json(FlagPage { items, next_cursor }))
}

async fn list_flags(
    db: &PgPool,
    q: &FlagQuery,
    cursor: Option<&Cursor>,
    limit: i64,
) -> Result<Vec<FlagRow>, AppError> {
    let rows = sqlx::query_as(
        "SELECT id, user_id, surface, severity, category, source, status, \
                strike_id IS NOT NULL AS has_strike, created_at \
         FROM moderation_flags \
         WHERE ($1::text IS NULL OR status = $1) \
           AND ($2::text IS NULL OR severity = $2) \
           AND ($3::timestamptz IS NULL OR (created_at, id) < ($3, $4)) \
         ORDER BY created_at DESC, id DESC LIMIT $5",
    )
    .bind(q.status.as_deref())
    .bind(q.severity.as_deref())
    .bind(cursor.map(|c| c.0))
    .bind(cursor.map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(db)
    .await
    .context("list flags")?;
    Ok(rows)
}

#[derive(Serialize)]
struct FlagDetail {
    id: Uuid,
    original: String,
    suggestion: Option<String>,
}

/// The audit row commits before the text leaves, so no read goes unrecorded.
async fn flag(
    State(state): State<AppState>,
    Owner(user): Owner,
    Path(id): Path<String>,
) -> Result<Json<FlagDetail>, AppError> {
    let id = parse_id(&id)?;
    let mut tx = begin(&state.db).await?;
    let row: Option<(Vec<u8>, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT original_enc, suggestion_enc FROM moderation_flags WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .context("read flag")?;
    let (original, suggestion) = row.ok_or(AppError::NotFound)?;
    let detail = FlagDetail {
        id,
        original: state.cipher.decrypt(&original)?,
        suggestion: suggestion.map(|s| state.cipher.decrypt(&s)).transpose()?,
    };
    audit::record(
        &mut tx,
        Some(user.id),
        "moderation.original_viewed",
        "moderation_flag",
        Some(id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Json(detail))
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Resolution {
    Uphold,
    Dismiss,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveBody {
    action: Resolution,
}

/// Locks the user, so a strike cannot land between the void and the recount.
async fn lock_user(conn: &mut PgConnection, user: Uuid) -> Result<(), AppError> {
    let found: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
            .bind(user)
            .fetch_optional(&mut *conn)
            .await
            .map_err(busy_err("moderation_busy", "lock user"))?;
    found.map(|_| ()).ok_or(AppError::NotFound)
}

async fn resolve(
    State(state): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
    AppJson(body): AppJson<ResolveBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let id = parse_id(&id)?;
    let mut tx = begin(&state.db).await?;
    let user: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM moderation_flags WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .context("read flag")?;
    let user = user.ok_or(AppError::NotFound)?;
    lock_user(&mut tx, user).await?;
    let (status, strike): (String, Option<i64>) =
        sqlx::query_as("SELECT status, strike_id FROM moderation_flags WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(busy_err("moderation_busy", "lock flag"))?;
    if status != "open" {
        return Err(AppError::Conflict("flag_resolved"));
    }
    let action = match body.action {
        Resolution::Uphold => "upheld",
        Resolution::Dismiss => "dismissed",
    };
    sqlx::query(
        "UPDATE moderation_flags SET status = $2, resolved_by = $3, resolved_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(action)
    .bind(staff.0.id)
    .execute(&mut *tx)
    .await
    .context("resolve flag")?;
    let voided = matches!(body.action, Resolution::Dismiss) && strike.is_some();
    if voided {
        sqlx::query(
            "UPDATE moderation_strikes SET voided_at = now(), voided_by = $2 \
             WHERE id = $1 AND voided_at IS NULL",
        )
        .bind(strike)
        .bind(staff.0.id)
        .execute(&mut *tx)
        .await
        .context("void strike")?;
        recalculate_mute(&mut tx, user).await?;
    }
    audit::record(
        &mut tx,
        Some(staff.0.id),
        "moderation.flag_resolved",
        "moderation_flag",
        Some(id),
        json!({ "status": action, "strike_voided": voided }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Json(json!({ "status": action })))
}

#[derive(Serialize, sqlx::FromRow)]
struct QueueRow {
    user_id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    owner_review_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    muted_until: Option<OffsetDateTime>,
    active_strikes: i64,
}

#[derive(Deserialize)]
struct QueueQuery {
    limit: Option<i64>,
}

async fn review_queue(
    State(state): State<AppState>,
    staff: Staff,
    AppQuery(q): AppQuery<QueueQuery>,
) -> Result<Json<Items<QueueRow>>, AppError> {
    let items: Vec<QueueRow> = sqlx::query_as(
        "SELECT m.user_id, m.owner_review_at, m.muted_until, \
                (SELECT count(*) FROM moderation_strikes s \
                 WHERE s.user_id = m.user_id AND s.voided_at IS NULL \
                   AND s.at > now() - make_interval(days => \
                       (SELECT strike_window_days FROM platform_settings WHERE id))) \
                AS active_strikes \
         FROM user_moderation m \
         WHERE m.owner_review_at IS NOT NULL AND m.banned_at IS NULL \
         ORDER BY m.owner_review_at LIMIT $1",
    )
    .bind(views::limit(q.limit)?)
    .fetch_all(&state.db)
    .await
    .context("read review queue")?;
    viewed(&state, staff.0.id, "moderation_review_queue", items.len()).await?;
    Ok(Json(Items { items }))
}

/// One owner action on a user: locks them, runs `sql`, refuses with `code` if
/// nothing changed, and audits.
async fn act_on_user(
    state: &AppState,
    actor: Uuid,
    user: Uuid,
    action: &str,
    detail: serde_json::Value,
    code: &'static str,
    sql: &str,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut tx = begin(&state.db).await?;
    lock_user(&mut tx, user).await?;
    let changed = sqlx::query(sql)
        .bind(user)
        .bind(actor)
        .execute(&mut *tx)
        .await
        .context("change posting state")?
        .rows_affected();
    if changed == 0 {
        return Err(AppError::Conflict(code));
    }
    audit::record(&mut tx, Some(actor), action, "user", Some(user), detail).await?;
    tx.commit().await.context("commit")?;
    Ok(Json(json!({ "user_id": user })))
}

async fn unmute(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    act_on_user(
        &state,
        owner.id,
        parse_id(&id)?,
        "moderation.unmuted",
        json!({}),
        "not_muted",
        "UPDATE user_moderation SET muted_until = NULL \
         WHERE user_id = $1 AND muted_until > now() AND $2::uuid IS NOT NULL",
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BanBody {
    reason: Option<String>,
}

async fn ban(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
    AppJson(body): AppJson<BanBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let reason = note("reason", body.reason.as_deref(), BAN_REASON_MAX)?;
    act_on_user(
        &state,
        owner.id,
        parse_id(&id)?,
        "moderation.banned",
        json!({ "reason": reason }),
        "already_banned",
        "INSERT INTO user_moderation (user_id, banned_at, banned_by) VALUES ($1, now(), $2) \
         ON CONFLICT (user_id) DO UPDATE SET banned_at = now(), banned_by = $2 \
         WHERE user_moderation.banned_at IS NULL",
    )
    .await
}

async fn unban(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    act_on_user(
        &state,
        owner.id,
        parse_id(&id)?,
        "moderation.unbanned",
        json!({}),
        "not_banned",
        "UPDATE user_moderation SET banned_at = NULL, banned_by = NULL \
         WHERE user_id = $1 AND banned_at IS NOT NULL AND $2::uuid IS NOT NULL",
    )
    .await
}

/// Locks the store first, so `recount` sees a settled set of reviews.
async fn set_hidden(
    state: &AppState,
    actor: Uuid,
    review: Uuid,
    hidden: bool,
) -> Result<Json<serde_json::Value>, AppError> {
    let mut tx = begin(&state.db).await?;
    let store: Option<Uuid> = sqlx::query_scalar("SELECT store_id FROM reviews WHERE id = $1")
        .bind(review)
        .fetch_optional(&mut *tx)
        .await
        .context("read review")?;
    let store = store.ok_or(AppError::NotFound)?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM stores WHERE id = $1 FOR NO KEY UPDATE")
        .bind(store)
        .fetch_one(&mut *tx)
        .await
        .map_err(busy_err("moderation_busy", "lock store"))?;
    let (sql, code, action) = if hidden {
        (
            "UPDATE reviews SET hidden_at = now(), hidden_by = $2 \
             WHERE id = $1 AND hidden_at IS NULL",
            "review_hidden",
            "review.hidden",
        )
    } else {
        (
            "UPDATE reviews SET hidden_at = NULL, hidden_by = NULL \
             WHERE id = $1 AND hidden_at IS NOT NULL AND $2::uuid IS NOT NULL",
            "review_not_hidden",
            "review.unhidden",
        )
    };
    let changed = sqlx::query(sql)
        .bind(review)
        .bind(actor)
        .execute(&mut *tx)
        .await
        .context("set review visibility")?
        .rows_affected();
    if changed == 0 {
        return Err(AppError::Conflict(code));
    }
    recount(&mut tx, store).await?;
    audit::record(
        &mut tx,
        Some(actor),
        action,
        "review",
        Some(review),
        json!({ "store_id": store }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Json(json!({ "id": review, "hidden": hidden })))
}

async fn hide(
    State(state): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    set_hidden(&state, staff.0.id, parse_id(&id)?, true).await
}

async fn unhide(
    State(state): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    set_hidden(&state, staff.0.id, parse_id(&id)?, false).await
}
