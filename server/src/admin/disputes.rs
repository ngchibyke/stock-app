//! Staff dispute routes: asking for the item back. The decision and the
//! staff screen are added here by later tasks.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Staff, note, parse_id};
use crate::{
    AppState,
    disputes::service::{self, PaidBy},
    error::AppError,
    extract::AppJson,
};

const NOTE_MAX: usize = 500;

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/disputes/{id}/request-return", post(request_return))
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
