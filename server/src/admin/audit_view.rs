//! Dashboard audit log view (plan 6, task 6). Owner only, read only: there is
//! no route that writes or deletes a row, and the table rejects UPDATE, DELETE
//! and TRUNCATE. `detail` is returned as stored; by the rules of
//! [`crate::audit::record`] it never holds personal text. Reading the log is
//! itself audited (`admin.viewed`).

use anyhow::Context;
use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{Owner, views};
use crate::{AppState, error::AppError, extract::AppQuery};

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/audit", get(list))
}

#[derive(Deserialize)]
struct ListQuery {
    actor_id: Option<Uuid>,
    action: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    from: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    to: Option<OffsetDateTime>,
    before: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct Row {
    id: i64,
    #[serde(with = "time::serde::rfc3339")]
    at: OffsetDateTime,
    actor_id: Option<Uuid>,
    action: String,
    subject_type: String,
    subject_id: Option<Uuid>,
    detail: Value,
}

#[derive(Serialize)]
struct Page {
    items: Vec<Row>,
    /// Pass as `before` for the next (older) page; null on the last page.
    next_cursor: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    Owner(owner): Owner,
    AppQuery(q): AppQuery<ListQuery>,
) -> Result<Json<Page>, AppError> {
    let limit = views::limit(q.limit)?;
    let mut items: Vec<Row> = sqlx::query_as(
        "SELECT id, at, actor_id, action, subject_type, subject_id, detail FROM audit_log \
         WHERE ($1::uuid IS NULL OR actor_id = $1) \
           AND ($2::text IS NULL OR action = $2) \
           AND ($3::timestamptz IS NULL OR at >= $3) \
           AND ($4::timestamptz IS NULL OR at < $4) \
           AND ($5::bigint IS NULL OR id < $5) \
         ORDER BY id DESC LIMIT $6",
    )
    .bind(q.actor_id)
    .bind(q.action.as_deref())
    .bind(q.from)
    .bind(q.to)
    .bind(q.before)
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await
    .context("list audit")?;
    let more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    let next_cursor = match (more, items.last()) {
        (true, Some(last)) => Some(last.id.to_string()),
        _ => None,
    };
    let mut conn = state.db.acquire().await.context("acquire")?;
    views::record_view(&mut conn, owner.id, "audit", items.len()).await?;
    Ok(Json(Page { items, next_cursor }))
}
