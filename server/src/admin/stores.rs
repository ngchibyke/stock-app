//! Dashboard stores (plan 6, task 3): search, view, suspend and unsuspend.
//!
//! Owner only. Suspending sets `stores.status = 'suspended'`, which already
//! hides the store and its products from browse, marks its cart lines
//! unavailable and refuses new checkouts for it. Orders already paid keep
//! running, and payouts are not frozen (an owner decision, see the roadmap).
//! Unsuspending returns the store to `active` when its payout account is
//! verified and to `pending` otherwise, the same rule activation follows.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{LOCK_TIMEOUT, Owner, note, parse_id, views};
use crate::{
    AppState, audit,
    checkout::service::busy_err,
    error::AppError,
    extract::{AppJson, AppQuery},
    market::placement,
    orders::views::Cursor,
};

const REASON_MAX: usize = 500;
const STATUSES: [&str; 3] = ["pending", "active", "suspended"];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/stores", get(list))
        .route("/admin/stores/{id}", get(view))
        .route("/admin/stores/{id}/suspend", post(suspend))
        .route("/admin/stores/{id}/unsuspend", post(unsuspend))
}

#[derive(Deserialize)]
struct ListQuery {
    q: Option<String>,
    status: Option<String>,
    before: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct StoreRow {
    id: Uuid,
    name: String,
    slug: String,
    owner_id: Uuid,
    status: String,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize)]
struct StorePage {
    items: Vec<StoreRow>,
    next_cursor: Option<String>,
}

/// A `LIKE` pattern for names starting with `q`, with `%`, `_` and `\` literal.
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
    Owner(owner): Owner,
    AppQuery(q): AppQuery<ListQuery>,
) -> Result<Json<StorePage>, AppError> {
    if let Some(s) = q.status.as_deref()
        && !STATUSES.contains(&s)
    {
        return Err(AppError::Validation(format!(
            "status must be one of {}",
            STATUSES.join(", ")
        )));
    }
    let limit = views::limit(q.limit)?;
    let cursor = q.before.as_deref().map(Cursor::decode).transpose()?;
    let like =
        q.q.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(name_prefix);

    let mut items: Vec<StoreRow> = sqlx::query_as(
        "SELECT id, name, slug, owner_id, status::text AS status, created_at FROM stores \
         WHERE deleted_at IS NULL \
           AND ($1::text IS NULL OR name ILIKE $1 ESCAPE '\\') \
           AND ($2::text IS NULL OR status::text = $2) \
           AND ($3::timestamptz IS NULL OR (created_at, id) < ($3, $4)) \
         ORDER BY created_at DESC, id DESC LIMIT $5",
    )
    .bind(like)
    .bind(q.status.as_deref())
    .bind(cursor.as_ref().map(|c| c.0))
    .bind(cursor.as_ref().map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await
    .context("list stores")?;
    let more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    let next_cursor = match (more, items.last()) {
        (true, Some(last)) => Some(Cursor(last.created_at, last.id).encode()?),
        _ => None,
    };
    let mut conn = state.db.acquire().await.context("acquire")?;
    views::record_view(&mut conn, owner.id, "stores", items.len()).await?;
    Ok(Json(StorePage { items, next_cursor }))
}

#[derive(Serialize, sqlx::FromRow)]
struct StoreDetail {
    id: Uuid,
    name: String,
    slug: String,
    owner_id: Uuid,
    status: String,
    suspended: bool,
    suspended_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    suspended_at: Option<OffsetDateTime>,
    payout_account_status: Option<String>,
    product_count: i64,
    open_order_count: i64,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

async fn detail(db: &PgPool, id: Uuid) -> Result<StoreDetail, AppError> {
    sqlx::query_as(
        "SELECT s.id, s.name, s.slug, s.owner_id, s.status::text AS status, \
                s.status = 'suspended' AS suspended, s.suspended_reason, s.suspended_at, \
                (SELECT pa.status::text FROM payout_accounts pa WHERE pa.store_id = s.id) \
                    AS payout_account_status, \
                (SELECT count(*) FROM products p \
                  WHERE p.store_id = s.id AND p.deleted_at IS NULL) AS product_count, \
                (SELECT count(*) FROM orders o WHERE o.store_id = s.id AND o.status IN \
                  ('paid', 'picked_up', 'out_for_delivery', 'delivered', 'refund_pending')) \
                    AS open_order_count, \
                s.created_at \
         FROM stores s WHERE s.id = $1 AND s.deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(db)
    .await
    .context("load store")?
    .ok_or(AppError::NotFound)
}

async fn view(
    State(state): State<AppState>,
    Owner(_): Owner,
    Path(id): Path<String>,
) -> Result<Json<StoreDetail>, AppError> {
    Ok(Json(detail(&state.db, parse_id(&id)?).await?))
}

#[derive(Deserialize)]
struct SuspendBody {
    reason: Option<String>,
}

/// Locks the store (lock order: stores) and returns its current status.
async fn lock_store(tx: &mut sqlx::PgConnection, id: Uuid) -> Result<String, AppError> {
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    sqlx::query_scalar(
        "SELECT status::text FROM stores WHERE id = $1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(busy_err("store_busy", "lock store"))?
    .ok_or(AppError::NotFound)
}

async fn suspend(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
    AppJson(body): AppJson<SuspendBody>,
) -> Result<Json<StoreDetail>, AppError> {
    let id = parse_id(&id)?;
    let reason = note("reason", body.reason.as_deref(), REASON_MAX)?;
    let mut tx = state.db.begin().await.context("begin")?;
    let current = lock_store(&mut tx, id).await?;
    if current == "suspended" {
        return Err(AppError::Conflict("already_suspended"));
    }
    sqlx::query(
        "UPDATE stores SET status = 'suspended', suspended_at = now(), \
                suspended_reason = $2, suspended_by = $3 WHERE id = $1",
    )
    .bind(id)
    .bind(&reason)
    .bind(owner.id)
    .execute(&mut *tx)
    .await
    .context("suspend store")?;
    audit::record(
        &mut tx,
        Some(owner.id),
        "admin.store_suspended",
        "store",
        Some(id),
        serde_json::json!({ "was": current, "reason": reason }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Json(detail(&state.db, id).await?))
}

async fn unsuspend(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<Json<StoreDetail>, AppError> {
    let id = parse_id(&id)?;
    let mut tx = state.db.begin().await.context("begin")?;
    let current = lock_store(&mut tx, id).await?;
    if current != "suspended" {
        return Err(AppError::Conflict("not_suspended"));
    }
    let verified: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM payout_accounts WHERE store_id = $1 AND status = 'verified')",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .context("payout account")?;
    let next = if verified { "active" } else { "pending" };
    sqlx::query(
        "UPDATE stores SET status = $2::store_status, suspended_at = NULL, \
                suspended_reason = NULL, suspended_by = NULL WHERE id = $1",
    )
    .bind(id)
    .bind(next)
    .execute(&mut *tx)
    .await
    .context("unsuspend store")?;
    if next == "active" {
        placement::place(&mut tx, &state.market, id).await?;
    }
    audit::record(
        &mut tx,
        Some(owner.id),
        "admin.store_unsuspended",
        "store",
        Some(id),
        serde_json::json!({ "now": next }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Json(detail(&state.db, id).await?))
}
