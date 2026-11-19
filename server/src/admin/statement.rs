//! Statement checks for the dashboard (plan 6, task 9). Owner only.
//!
//! `GET /admin/statement-checks` lists the latest checks, newest first.
//! `POST /admin/statement-checks` with `{"day": "2026-11-18"}` checks a finished
//! Lagos day now. 503 `statement_unavailable` when no statement source is
//! installed (see [`crate::statement`]); 422 `day_not_finished` for today or a
//! future day. A provider that cannot answer is recorded as `unavailable`.

use anyhow::Context;
use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::{Date, OffsetDateTime, macros::format_description};
use uuid::Uuid;

use super::{Owner, views};
use crate::{
    AppState, audit,
    error::AppError,
    extract::{AppJson, AppQuery},
    statement::check_day,
};

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/statement-checks", get(list).post(run))
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct CheckRow {
    id: Uuid,
    day: String,
    source: String,
    status: String,
    ledger: Option<Value>,
    statement: Option<Value>,
    diff: Option<Value>,
    tolerance_kobo: i64,
    #[serde(with = "time::serde::rfc3339")]
    checked_at: OffsetDateTime,
}

async fn list(
    State(state): State<AppState>,
    Owner(owner): Owner,
    AppQuery(q): AppQuery<ListQuery>,
) -> Result<Json<views::Items<CheckRow>>, AppError> {
    let limit = views::limit(q.limit)?;
    let items: Vec<CheckRow> = sqlx::query_as(
        "SELECT id, day::text AS day, source, status, ledger, statement, diff, tolerance_kobo, checked_at \
         FROM statement_checks ORDER BY checked_at DESC, id DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .context("list statement checks")?;
    let mut conn = state.db.acquire().await.context("acquire")?;
    views::record_view(&mut conn, owner.id, "statement_checks", items.len()).await?;
    Ok(Json(views::Items { items }))
}

#[derive(Deserialize)]
struct RunBody {
    day: String,
}

async fn run(
    State(state): State<AppState>,
    Owner(owner): Owner,
    AppJson(body): AppJson<RunBody>,
) -> Result<Json<Value>, AppError> {
    let day = Date::parse(&body.day, format_description!("[year]-[month]-[day]"))
        .map_err(|_| AppError::Validation("day must be a date like 2026-11-18".into()))?;
    let today: Date = sqlx::query_scalar("SELECT (now() AT TIME ZONE 'Africa/Lagos')::date")
        .fetch_one(&state.db)
        .await
        .context("today in Lagos")?;
    if day >= today {
        return Err(AppError::Unprocessable(
            "day_not_finished",
            "A day can be checked once it has ended (Lagos time)".into(),
        ));
    }
    let Some(source) = state.statements.as_deref() else {
        return Err(AppError::Unavailable("statement_unavailable"));
    };
    let outcome = check_day(&state.db, source, day, Some(owner.id)).await?;
    let mut conn = state.db.acquire().await.context("acquire")?;
    audit::record(
        &mut conn,
        Some(owner.id),
        "admin.statement_checked",
        "statement_check",
        None,
        serde_json::json!({ "day": day.to_string(), "status": outcome.as_str() }),
    )
    .await?;
    let latest: CheckRow = sqlx::query_as(
        "SELECT id, day::text AS day, source, status, ledger, statement, diff, tolerance_kobo, checked_at \
         FROM statement_checks WHERE day = $1 ORDER BY checked_at DESC, id DESC LIMIT 1",
    )
    .bind(day)
    .fetch_one(&state.db)
    .await
    .context("load statement check")?;
    Ok(Json(serde_json::to_value(latest).context("serialize")?))
}
