use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use uuid::Uuid;

use super::service::{self, Created, Finalized, UploadRequest};
use crate::{AppState, auth::AuthUser, error::AppError, extract::AppJson};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/uploads", post(create))
        .route("/uploads/{id}/finalize", post(finalize))
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<UploadRequest>,
) -> Result<(StatusCode, Json<Created>), AppError> {
    Ok((
        StatusCode::CREATED,
        Json(service::create(&state, &user, body).await?),
    ))
}

async fn finalize(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Finalized>, AppError> {
    let id: Uuid = id.parse().map_err(|_| AppError::NotFound)?;
    Ok(Json(service::finalize(&state, user.id, id).await?))
}
