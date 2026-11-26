//! `/me/deletion`, `/me/export`, `/me/export/file` and the Owner's `/admin/retention`.

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;

use super::{
    deletion::{self, DeletionStatus},
    export::{self, ExportStatus},
    retention::{self, Patch, Windows},
};
use crate::{
    AppState,
    auth::{AuthUser, Role},
    error::AppError,
    extract::AppJson,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/me/deletion",
            get(deletion_status)
                .post(deletion_request)
                .delete(deletion_cancel),
        )
        .route("/me/export", get(export_status).post(export_request))
        .route("/me/export/file", get(export_file))
        .route("/admin/retention", get(windows).patch(set_windows))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirm {
    password: String,
}

async fn deletion_status(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<DeletionStatus>, AppError> {
    Ok(Json(deletion::status(&state.db, user.id).await?))
}

async fn deletion_request(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<Confirm>,
) -> Result<Json<DeletionStatus>, AppError> {
    Ok(Json(
        deletion::request(&state.db, &user, &body.password).await?,
    ))
}

async fn deletion_cancel(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<StatusCode, AppError> {
    deletion::cancel(&state.db, user.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn export_status(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<ExportStatus>, AppError> {
    Ok(Json(export::status(&state.db, user.id).await?))
}

async fn export_request(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<Confirm>,
) -> Result<(StatusCode, Json<ExportStatus>), AppError> {
    let status = export::request(&state.db, user.id, &body.password).await?;
    Ok((StatusCode::ACCEPTED, Json(status)))
}

async fn export_file(State(state): State<AppState>, user: AuthUser) -> Result<Response, AppError> {
    let bytes = export::file(&state, user.id).await?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"my-data.json\""),
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn windows(State(state): State<AppState>, user: AuthUser) -> Result<Json<Windows>, AppError> {
    user.require_role(Role::Owner)?;
    Ok(Json(retention::load(&state.db).await?))
}

async fn set_windows(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(patch): AppJson<Patch>,
) -> Result<Json<Windows>, AppError> {
    user.require_role(Role::Owner)?;
    Ok(Json(retention::update(&state.db, user.id, patch).await?))
}
