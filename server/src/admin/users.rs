//! Dashboard user search and view (plan 6, task 1).
//!
//! The list never carries a phone: it shows ids, display names, roles and
//! moderation state. Support sees only the people in an open dispute (spec
//! §11: personal data only inside an open dispute). The single view decrypts
//! the phone, is Owner-only, and writes an `admin.user_read` audit row.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    Owner, Staff, parse_id,
    views::{self},
};
use crate::{
    AppState, audit, crypto::blind_index, error::AppError, extract::AppQuery,
    orders::views::Cursor, phone::normalize_ng_phone,
};

const ROLES: [&str; 3] = ["user", "support", "owner"];
const STATES: [&str; 4] = ["active", "banned", "muted", "deleted"];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/users", get(list))
        .route("/admin/users/{id}", get(view))
}

#[derive(Deserialize)]
struct ListQuery {
    q: Option<String>,
    role: Option<String>,
    state: Option<String>,
    before: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    display_name: String,
    role: String,
    is_seller: bool,
    banned: bool,
    muted: bool,
    deleted: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize)]
struct UserPage {
    items: Vec<UserRow>,
    next_cursor: Option<String>,
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

/// Whether the search text is a phone number (digits and the usual
/// separators, at least five digits) rather than a name.
fn looks_like_phone(q: &str) -> bool {
    q.chars().all(|c| c.is_ascii_digit() || "+- ".contains(c))
        && q.chars().filter(char::is_ascii_digit).count() >= 5
}

/// A `LIKE` pattern matching names that start with `q`, `%`, `_` and `\` literal.
fn name_prefix(q: &str) -> String {
    let mut p = String::new();
    for c in q.trim().chars() {
        if matches!(c, '%' | '_' | '\\') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

async fn list(
    State(state): State<AppState>,
    staff: Staff,
    AppQuery(q): AppQuery<ListQuery>,
) -> Result<Json<UserPage>, AppError> {
    one_of("role", q.role.as_deref(), &ROLES)?;
    one_of("state", q.state.as_deref(), &STATES)?;
    let limit = views::limit(q.limit)?;
    let cursor = q.before.as_deref().map(Cursor::decode).transpose()?;

    let text = q.q.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let (phone_idx, name_like) = match text {
        Some(t) if looks_like_phone(t) => (
            Some(blind_index(&state.index_key, &normalize_ng_phone(t)?)),
            None,
        ),
        Some(t) => (None, Some(name_prefix(t))),
        None => (None, None),
    };

    let mut items = search(
        &state.db,
        phone_idx,
        name_like,
        &q,
        cursor.as_ref(),
        limit,
        !staff.is_owner(),
    )
    .await?;
    let more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    let next_cursor = match (more, items.last()) {
        (true, Some(last)) => Some(Cursor(last.created_at, last.id).encode()?),
        _ => None,
    };
    let mut conn = state.db.acquire().await.context("acquire")?;
    views::record_view(&mut conn, staff.0.id, "users", items.len()).await?;
    Ok(Json(UserPage { items, next_cursor }))
}

async fn search(
    db: &PgPool,
    phone_idx: Option<Vec<u8>>,
    name_like: Option<String>,
    q: &ListQuery,
    cursor: Option<&Cursor>,
    limit: i64,
    dispute_parties_only: bool,
) -> Result<Vec<UserRow>, AppError> {
    let rows = sqlx::query_as(
        "SELECT u.id, u.display_name, u.role::text AS role, u.is_seller, \
                COALESCE(m.banned_at IS NOT NULL, false) AS banned, \
                COALESCE(m.muted_until > now(), false) AS muted, \
                u.deleted_at IS NOT NULL AS deleted, u.created_at \
         FROM users u LEFT JOIN user_moderation m ON m.user_id = u.id \
         WHERE ($1::bytea IS NULL OR u.phone_idx = $1) \
           AND ($2::text IS NULL OR u.display_name ILIKE $2 ESCAPE '\\') \
           AND ($3::text IS NULL OR u.role::text = $3) \
           AND (CASE $4::text \
                  WHEN 'banned' THEN m.banned_at IS NOT NULL \
                  WHEN 'muted' THEN m.muted_until > now() \
                  WHEN 'deleted' THEN u.deleted_at IS NOT NULL \
                  WHEN 'active' THEN u.deleted_at IS NULL AND m.banned_at IS NULL \
                  ELSE true END) \
           AND (NOT $5 OR u.id IN ( \
                  SELECT d.buyer_id FROM disputes d \
                   WHERE d.status IN ('awaiting_seller', 'awaiting_return', 'awaiting_decision') \
                  UNION \
                  SELECT s.owner_id FROM disputes d JOIN stores s ON s.id = d.store_id \
                   WHERE d.status IN ('awaiting_seller', 'awaiting_return', 'awaiting_decision'))) \
           AND ($6::timestamptz IS NULL OR (u.created_at, u.id) < ($6, $7)) \
         ORDER BY u.created_at DESC, u.id DESC LIMIT $8",
    )
    .bind(phone_idx)
    .bind(name_like)
    .bind(q.role.as_deref())
    .bind(q.state.as_deref())
    .bind(dispute_parties_only)
    .bind(cursor.map(|c| c.0))
    .bind(cursor.map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(db)
    .await
    .context("search users")?;
    Ok(rows)
}

#[derive(Serialize)]
struct UserDetail {
    id: Uuid,
    display_name: String,
    phone: String,
    role: String,
    is_seller: bool,
    banned: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    banned_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    muted_until: Option<OffsetDateTime>,
    strike_count: i64,
    store_ids: Vec<Uuid>,
    orders_as_buyer: i64,
    deleted: bool,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct DetailRow {
    id: Uuid,
    display_name: String,
    phone_enc: Vec<u8>,
    role: String,
    is_seller: bool,
    banned_at: Option<OffsetDateTime>,
    muted_until: Option<OffsetDateTime>,
    deleted: bool,
    created_at: OffsetDateTime,
}

async fn view(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<Json<UserDetail>, AppError> {
    let id = parse_id(&id)?;
    let row: DetailRow = sqlx::query_as(
        "SELECT u.id, u.display_name, u.phone_enc, u.role::text AS role, u.is_seller, \
                m.banned_at, m.muted_until, u.deleted_at IS NOT NULL AS deleted, u.created_at \
         FROM users u LEFT JOIN user_moderation m ON m.user_id = u.id WHERE u.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .context("load user")?
    .ok_or(AppError::NotFound)?;
    let (strike_count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM moderation_strikes WHERE user_id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .context("count strikes")?;
    let store_ids: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM stores WHERE owner_id = $1 ORDER BY created_at")
            .bind(id)
            .fetch_all(&state.db)
            .await
            .context("list stores")?;
    let (orders_as_buyer,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM orders WHERE buyer_id = $1")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .context("count orders")?;

    let phone = state.cipher.decrypt(&row.phone_enc)?;
    let mut conn = state.db.acquire().await.context("acquire")?;
    audit::record(
        &mut conn,
        Some(owner.id),
        "admin.user_read",
        "user",
        Some(id),
        serde_json::json!({}),
    )
    .await?;
    Ok(Json(UserDetail {
        id: row.id,
        display_name: row.display_name,
        phone,
        role: row.role,
        is_seller: row.is_seller,
        banned: row.banned_at.is_some(),
        banned_at: row.banned_at,
        muted_until: row.muted_until,
        strike_count,
        store_ids: store_ids.into_iter().map(|(i,)| i).collect(),
        orders_as_buyer,
        deleted: row.deleted,
        created_at: row.created_at,
    }))
}
