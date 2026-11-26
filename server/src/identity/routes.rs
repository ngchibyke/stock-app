//! `POST /me/identity/start`, `GET /me/identity` and the provider's webhook
//! `POST /webhooks/identity/{provider}`.
//!
//! The NIN travels only in the start request's body. The webhook is unauthenticated:
//! the signature is checked before the body is read, a name other than the configured
//! provider's is 404, and the answer is 200 whatever was decided so the provider does
//! not retry a settled message. Logging: never the body, headers or a name.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode},
    middleware::map_response,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use tower_http::limit::RequestBodyLimitLayer;

use super::{IdentityError, service};
use crate::{
    AppState,
    auth::AuthUser,
    error::AppError,
    extract::AppJson,
    payments::webhooks::{MAX_WEBHOOK_BODY, envelope_413},
};

pub fn router() -> Router<AppState> {
    let hook = Router::new()
        .route("/webhooks/identity/{provider}", post(receive))
        .layer(RequestBodyLimitLayer::new(MAX_WEBHOOK_BODY))
        .layer(map_response(envelope_413));
    Router::new()
        .route("/me/identity/start", post(start))
        .route("/me/identity", get(status))
        .merge(hook)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartBody {
    nin: String,
}

async fn start(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<StartBody>,
) -> Result<Json<service::StartView>, AppError> {
    Ok(Json(service::start(&state, &user, &body.nin).await?))
}

async fn status(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<service::StatusView>, AppError> {
    Ok(Json(service::status(&state, user.id).await?))
}

fn answer(s: &'static str) -> Response {
    (StatusCode::OK, Json(json!({ "status": s }))).into_response()
}

async fn receive(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let provider = state
        .identity
        .as_deref()
        .filter(|p| p.name() == provider)
        .ok_or(AppError::NotFound)?;
    let body = body.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::PayloadTooLarge
        } else {
            AppError::Validation("Request body could not be read".into())
        }
    })?;
    let event = match provider.parse_webhook(&headers, &body) {
        Ok(event) => event,
        Err(IdentityError::InvalidSignature) => {
            tracing::warn!(
                provider = provider.name(),
                "identity webhook with invalid signature"
            );
            return Err(AppError::InvalidSignature);
        }
        Err(_) => {
            tracing::info!(provider = provider.name(), "identity webhook ignored");
            return Ok(answer("ignored"));
        }
    };
    match service::apply_by_ref(&state, provider.name(), &event.provider_ref, &event.outcome)
        .await?
    {
        service::Applied::Done => Ok(answer("received")),
        service::Applied::Ignored => Ok(answer("ignored")),
    }
}
