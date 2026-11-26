//! Dashboard side of the seller identity check: a user's history, the review queue, the
//! owner's pass/fail decisions and revokes.
//!
//! Support sees statuses and reason codes only. The verified name, year of birth, provider
//! reference and the owner's decision notes are Owner-only, and every read of a name writes
//! an `admin.identity_read` audit row. Decisions and revokes are Owner-only and need a
//! reason, which is kept on the check row and never in the audit log.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    Owner, Staff, note, parse_id,
    views::{self, Items},
};
use crate::{
    AppState, audit,
    auth::Role,
    error::AppError,
    extract::{AppJson, AppQuery},
    identity::service::{self, Decision},
};

const NOTE_MAX: usize = 500;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/identity/review-queue", get(queue))
        .route("/admin/users/{id}/identity", get(history))
        .route("/admin/identity/{id}/decide", post(decide))
        .route("/admin/identity/{id}/revoke", post(revoke))
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    user_id: Uuid,
    provider: String,
    provider_ref: Option<String>,
    status: String,
    reason: Option<String>,
    full_name_enc: Option<Vec<u8>>,
    birth_year: Option<i32>,
    started_at: OffsetDateTime,
    decided_at: Option<OffsetDateTime>,
    decided_by: Option<Uuid>,
    decision_note: Option<String>,
}

const COLUMNS: &str = "id, user_id, provider, provider_ref, status, reason, full_name_enc, \
                       birth_year, started_at, decided_at, decided_by, decision_note";

#[derive(Serialize)]
struct CheckView {
    id: Uuid,
    user_id: Uuid,
    status: String,
    reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    decided_at: Option<OffsetDateTime>,
    decided_by: Option<Uuid>,
    // Owner only; absent for Support.
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    birth_year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision_note: Option<String>,
}

fn view(state: &AppState, row: Row, owner: bool) -> Result<CheckView, AppError> {
    let verified_name = match (&row.full_name_enc, owner) {
        (Some(blob), true) => Some(state.cipher.decrypt(blob)?),
        _ => None,
    };
    Ok(CheckView {
        id: row.id,
        user_id: row.user_id,
        status: row.status,
        reason: row.reason,
        started_at: row.started_at,
        decided_at: row.decided_at,
        decided_by: row.decided_by,
        provider: owner.then_some(row.provider),
        provider_ref: if owner { row.provider_ref } else { None },
        verified_name,
        birth_year: if owner { row.birth_year } else { None },
        decision_note: if owner { row.decision_note } else { None },
    })
}

#[derive(Deserialize)]
struct QueueQuery {
    limit: Option<i64>,
}

async fn queue(
    State(state): State<AppState>,
    staff: Staff,
    AppQuery(q): AppQuery<QueueQuery>,
) -> Result<Json<Items<CheckView>>, AppError> {
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM identity_checks WHERE status = 'review' \
         ORDER BY started_at LIMIT $1"
    ))
    .bind(views::limit(q.limit)?)
    .fetch_all(&state.db)
    .await
    .context("read identity review queue")?;
    let owner = staff.0.role == Role::Owner;
    finish(&state, staff.0.id, "identity_review_queue", rows, owner).await
}

async fn history(
    State(state): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
) -> Result<Json<Items<CheckView>>, AppError> {
    let user = parse_id(&id)?;
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM identity_checks WHERE user_id = $1 \
         ORDER BY started_at DESC LIMIT 50"
    ))
    .bind(user)
    .fetch_all(&state.db)
    .await
    .context("read identity history")?;
    let owner = staff.0.role == Role::Owner;
    finish(&state, staff.0.id, "identity_history", rows, owner).await
}

async fn finish(
    state: &AppState,
    actor: Uuid,
    name: &str,
    rows: Vec<Row>,
    owner: bool,
) -> Result<Json<Items<CheckView>>, AppError> {
    let read_names: Vec<Uuid> = if owner {
        rows.iter()
            .filter(|r| r.full_name_enc.is_some())
            .map(|r| r.id)
            .collect()
    } else {
        Vec::new()
    };
    let mut conn = state.db.acquire().await.context("acquire")?;
    // The audit row commits before any name leaves.
    for id in &read_names {
        audit::record(
            &mut conn,
            Some(actor),
            "admin.identity_read",
            "identity_check",
            Some(*id),
            serde_json::json!({}),
        )
        .await?;
    }
    views::record_view(&mut conn, actor, name, rows.len()).await?;
    let items = rows
        .into_iter()
        .map(|r| view(state, r, owner))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(Items { items }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideBody {
    decision: String,
    reason: Option<String>,
}

async fn decide(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
    AppJson(body): AppJson<DecideBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let id = parse_id(&id)?;
    let reason = note("reason", body.reason.as_deref(), NOTE_MAX)?;
    let decision = match body.decision.as_str() {
        "pass" => Decision::Pass,
        "fail" => Decision::Fail,
        _ => {
            return Err(AppError::Validation("decision must be pass or fail".into()));
        }
    };
    service::decide(&state, owner.id, id, decision, &reason).await?;
    Ok(Json(serde_json::json!({ "id": id })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeBody {
    reason: Option<String>,
}

async fn revoke(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
    AppJson(body): AppJson<RevokeBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let id = parse_id(&id)?;
    let reason = note("reason", body.reason.as_deref(), NOTE_MAX)?;
    service::revoke(&state, owner.id, id, &reason).await?;
    Ok(Json(serde_json::json!({ "id": id })))
}
