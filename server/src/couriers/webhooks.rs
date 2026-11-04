//! `POST /webhooks/couriers/{provider}`: unauthenticated courier callbacks.
//!
//! - The body is raw bytes, capped at 64 KiB on this route only.
//! - A name other than the configured courier's -> 404.
//! - The signature is checked before the body is parsed: mismatch -> 401
//!   `invalid_signature`, nothing written.
//! - A status update goes through
//!   [`apply_courier_update`], which records the event and applies it. The
//!   answer is 200 whatever it decided, including an unknown `provider_order_id`.
//!   A busy order lock is 503 `order_busy`, so the provider retries.
//! - A wallet warning writes the audit row behind the owner's `wallet_alerts`,
//!   with the event name only.
//! - A signed body we cannot read, and events we do not act on -> 200 `ignored`.
//!
//! The provider wants an answer within seconds: no provider call happens here.
//! Logging: never the body or headers, which carry phones.

use anyhow::Context;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode},
    middleware::map_response,
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::json;
use tower_http::limit::RequestBodyLimitLayer;

use super::{CourierError, CourierEvent};
use crate::{
    AppState, audit,
    delivery::{
        events::{Applied, apply_courier_update},
        shipments::WALLET_ALERT,
    },
    error::AppError,
    payments::webhooks::{MAX_WEBHOOK_BODY, envelope_413},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/webhooks/couriers/{provider}", post(receive))
        .layer(RequestBodyLimitLayer::new(MAX_WEBHOOK_BODY))
        // Outermost, so it also sees the limit layer's own 413.
        .layer(map_response(envelope_413))
}

fn status(s: &'static str) -> Response {
    (StatusCode::OK, Json(json!({ "status": s }))).into_response()
}

async fn receive(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let courier = state
        .courier
        .as_deref()
        .filter(|c| c.name() == provider)
        .ok_or(AppError::NotFound)?;
    let body = body.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::PayloadTooLarge
        } else {
            AppError::Validation("Request body could not be read".into())
        }
    })?;
    let name = courier.name();
    let event = match courier.parse_webhook(&headers, &body) {
        Ok(event) => event,
        Err(CourierError::InvalidSignature) => {
            tracing::warn!(provider = name, "courier webhook with invalid signature");
            return Err(AppError::InvalidSignature);
        }
        Err(_) => {
            tracing::info!(
                provider = name,
                kind = "malformed",
                "courier webhook ignored"
            );
            return Ok(status("ignored"));
        }
    };
    match event {
        CourierEvent::Update(update) => match apply_courier_update(&state, update).await? {
            Applied::Updated | Applied::Recorded => Ok(status("received")),
            _ => Ok(status("ignored")),
        },
        CourierEvent::WalletAlert { event } => {
            let mut tx = state.db.begin().await.context("begin")?;
            audit::record(
                &mut tx,
                None,
                WALLET_ALERT,
                "courier",
                None,
                json!({ "event": event }),
            )
            .await?;
            tx.commit().await.context("commit wallet alert")?;
            Ok(status("received"))
        }
        _ => {
            tracing::info!(provider = name, kind = "other", "courier webhook ignored");
            Ok(status("ignored"))
        }
    }
}
