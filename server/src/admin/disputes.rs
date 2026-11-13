//! Staff dispute routes: asking for the item back, and the owner's decision.
//! The staff queue and screen (read-only) are served here too.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Owner, Staff, note, parse_id, views};
use crate::{
    AppState,
    disputes::{
        decide::{self, Decision},
        service::{self, DisputeStatus, DisputeView, PaidBy},
        staff::{self, QueueRow, StaffDisputeView},
    },
    error::AppError,
    extract::{AppJson, AppQuery},
};

const NOTE_MAX: usize = 500;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/disputes", get(queue))
        .route("/admin/disputes/{id}", get(screen))
        .route("/admin/disputes/{id}/request-return", post(request_return))
        .route("/admin/disputes/{id}/decide", post(decide))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideBody {
    decision: String,
    refund_kobo: Option<i64>,
    note: Option<String>,
}

/// Owner only (Support gets 403 before the body is read). `refund_kobo` is
/// required for `partial_refund` and refused otherwise.
async fn decide(
    State(state): State<AppState>,
    Owner(user): Owner,
    Path(id): Path<String>,
    AppJson(body): AppJson<DecideBody>,
) -> Result<Json<DisputeView>, AppError> {
    let id = parse_id(&id)?;
    let decision = match (body.decision.as_str(), body.refund_kobo) {
        ("partial_refund", Some(kobo)) => Decision::PartialRefund(kobo),
        ("partial_refund", None) => {
            return Err(AppError::Validation(
                "refund_kobo is required for partial_refund".into(),
            ));
        }
        ("full_refund" | "release", Some(_)) => {
            return Err(AppError::Validation(
                "refund_kobo is only allowed for partial_refund".into(),
            ));
        }
        ("full_refund", None) => Decision::FullRefund,
        ("release", None) => Decision::Release,
        _ => {
            return Err(AppError::Validation(
                "decision must be full_refund, partial_refund or release".into(),
            ));
        }
    };
    let note = note("note", body.note.as_deref(), NOTE_MAX)?;
    Ok(Json(
        decide::decide(&state, user.id, id, decision, &note).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReturnBody {
    paid_by: String,
    note: Option<String>,
}

async fn request_return(
    State(state): State<AppState>,
    Staff(user): Staff,
    Path(id): Path<String>,
    AppJson(body): AppJson<ReturnBody>,
) -> Result<Json<Value>, AppError> {
    let id = parse_id(&id)?;
    let paid_by = PaidBy::parse(&body.paid_by)
        .ok_or_else(|| AppError::Validation("paid_by must be buyer or seller".into()))?;
    let note = note("note", body.note.as_deref(), NOTE_MAX)?;
    service::request_return(&state, user.id, id, paid_by, &note).await?;
    Ok(Json(
        json!({ "status": "awaiting_return", "paid_by": paid_by.as_str() }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueQuery {
    status: Option<String>,
    limit: Option<i64>,
}

/// Oldest first. Ids, reasons and amounts only; the read is audited.
async fn queue(
    State(state): State<AppState>,
    Staff(user): Staff,
    AppQuery(q): AppQuery<QueueQuery>,
) -> Result<Json<views::Items<QueueRow>>, AppError> {
    let status = q
        .status
        .as_deref()
        .map(|s| {
            DisputeStatus::parse(s)
                .ok_or_else(|| AppError::Validation("status is not a dispute status".into()))
        })
        .transpose()?;
    let limit = views::limit(q.limit)?;
    let items = staff::queue(&state.db, status, limit).await?;
    let mut conn = state.db.acquire().await.context("acquire")?;
    views::record_view(&mut conn, user.id, "disputes", items.len()).await?;
    Ok(Json(views::Items { items }))
}

/// Owner: any dispute. Support: open disputes, without originals.
async fn screen(
    State(state): State<AppState>,
    staff: Staff,
    Path(id): Path<String>,
) -> Result<Json<StaffDisputeView>, AppError> {
    let id = parse_id(&id)?;
    Ok(Json(staff::screen(&state, &staff, id).await?))
}
