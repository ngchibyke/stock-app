//! `POST /uploads`: one photo in, a key out.
//!
//! The body is the image itself (`Content-Type: image/jpeg`, `image/png` or
//! `image/webp`), at most 5 MB. The answer is `{ key, content_type, bytes, width,
//! height }`; the key goes in a product, review or dispute statement, and only
//! the uploader may attach it. 503 `uploads_unavailable` when no object store is
//! configured, 429 `too_many_uploads` past 60 an hour, 413 for a body over 5 MB.

use anyhow::Context;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State, rejection::BytesRejection},
    http::StatusCode,
    routing::post,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    process::{MAX_BYTES, process},
    service,
};
use crate::{AppState, auth::AuthUser, error::AppError};

pub fn router() -> Router<AppState> {
    // A little over the limit, so an oversize body is answered by our own check.
    Router::new().route(
        "/uploads",
        post(upload).layer(DefaultBodyLimit::max(MAX_BYTES + 4096)),
    )
}

async fn upload(
    State(state): State<AppState>,
    user: AuthUser,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let Some(store) = state.photos.clone() else {
        return Err(AppError::Unavailable("uploads_unavailable"));
    };
    let body = body.map_err(|_| AppError::PayloadTooLarge)?;
    if body.len() > MAX_BYTES {
        return Err(AppError::PayloadTooLarge);
    }
    if service::recent_count(&state.db, user.id).await? >= service::MAX_UPLOADS_PER_HOUR {
        return Err(AppError::TooManyRequests("too_many_uploads"));
    }
    let processed = tokio::task::spawn_blocking(move || process(&body))
        .await
        .context("image task")??;
    let key = format!("photos/{}.{}", Uuid::new_v4(), processed.extension);
    store
        .put(&key, processed.bytes.clone(), processed.content_type)
        .await
        .map_err(|_| AppError::Unavailable("uploads_unavailable"))?;
    service::record(
        &state.db,
        &key,
        user.id,
        processed.content_type,
        processed.bytes.len(),
        processed.width,
        processed.height,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "key": key,
            "content_type": processed.content_type,
            "bytes": processed.bytes.len(),
            "width": processed.width,
            "height": processed.height,
        })),
    ))
}
