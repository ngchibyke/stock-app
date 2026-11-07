//! Dispute routes.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;

use super::service::{self, DisputeView, Reason, Statement, WithdrawResponse};
use crate::{
    AppState, admin::parse_id, auth::AuthUser, error::AppError, extract::AppJson,
    orders::routes::confirmed,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/orders/{id}/dispute", post(open))
        .route("/disputes/{id}", get(show))
        .route("/disputes/{id}/withdraw", post(withdraw))
        .route("/disputes/{id}/response", post(respond))
        .route("/disputes/{id}/return-received", post(return_received))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenBody {
    reason: String,
    body: String,
    #[serde(default)]
    photo_keys: Vec<String>,
}

async fn open(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    AppJson(body): AppJson<OpenBody>,
) -> Result<(StatusCode, Json<DisputeView>), AppError> {
    let reason = Reason::parse(&body.reason)
        .ok_or_else(|| AppError::Validation("reason is not one of the allowed values".into()))?;
    let statement = Statement {
        body: body.body,
        photo_keys: body.photo_keys,
    };
    let view = service::open(&state, user.id, parse_id(&id)?, reason, statement).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<DisputeView>, AppError> {
    Ok(Json(service::view(&state, user.id, parse_id(&id)?).await?))
}

async fn withdraw(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<WithdrawResponse>, AppError> {
    let id = parse_id(&id)?;
    // Ownership first, so a stranger gets 404 whatever the body says.
    service::view(&state, user.id, id).await?;
    if !confirmed(&body) {
        return Err(AppError::Unprocessable(
            "confirmation_required",
            "Confirm that you want to withdraw this report.".into(),
        ));
    }
    Ok(Json(service::withdraw(&state, user.id, id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RespondBody {
    body: String,
    #[serde(default)]
    photo_keys: Vec<String>,
}

async fn respond(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    AppJson(body): AppJson<RespondBody>,
) -> Result<Json<DisputeView>, AppError> {
    let statement = Statement {
        body: body.body,
        photo_keys: body.photo_keys,
    };
    Ok(Json(
        service::respond(&state, user.id, parse_id(&id)?, statement).await?,
    ))
}

async fn return_received(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<DisputeView>, AppError> {
    Ok(Json(
        service::return_received(&state, user.id, parse_id(&id)?).await?,
    ))
}
