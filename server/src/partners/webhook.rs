//! `POST /webhooks/partner`: signed events from the partner. The signature is
//! HMAC-SHA256 over `{timestamp}.{raw body}` (hex, in `X-Partner-Signature`), with the
//! timestamp in unix seconds in `X-Partner-Timestamp`.
//!
//! The signature and the timestamp are checked before the body is read as JSON; an old or
//! future timestamp (more than five minutes either way) is refused, so a captured request
//! cannot be replayed later. A body over 64 KiB is refused. Each `event_id` is stored once:
//! a repeat is answered 200 and changes nothing. Logging: never the body or the headers.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode},
    middleware::map_response,
    routing::post,
};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tower_http::limit::RequestBodyLimitLayer;

use crate::{
    AppState,
    error::AppError,
    payments::webhooks::{MAX_WEBHOOK_BODY, envelope_413},
};

pub const SIGNATURE_HEADER: &str = "x-partner-signature";
pub const TIMESTAMP_HEADER: &str = "x-partner-timestamp";
pub const WINDOW_SECS: i64 = 300;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/webhooks/partner", post(receive))
        .layer(RequestBodyLimitLayer::new(MAX_WEBHOOK_BODY))
        .layer(map_response(envelope_413))
}

/// The hex signature for `body` at `timestamp` under `secret` (tests and the partner's
/// documentation use the same recipe).
pub fn sign(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn verify(secret: &str, headers: &HeaderMap, body: &[u8], now: i64) -> bool {
    let ts: Option<i64> = headers
        .get(TIMESTAMP_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let sig = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(hex_decode);
    let (Some(ts), Some(sig)) = (ts, sig) else {
        return false;
    };
    if (now - ts).abs() > WINDOW_SECS {
        return false;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(ts.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    // Constant time.
    mac.verify_slice(&sig).is_ok()
}

async fn receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let secret = state
        .partner
        .webhook_secret()
        .ok_or(AppError::NotFound)?
        .expose()
        .to_string();
    let body = body.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::PayloadTooLarge
        } else {
            AppError::Validation("Request body could not be read".into())
        }
    })?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    if !verify(&secret, &headers, &body, now) {
        tracing::warn!("partner webhook refused: bad signature or timestamp");
        return Err(AppError::InvalidSignature);
    }
    let v: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::Validation("Request body is not valid".into()))?;
    let (Some(event_id), Some(kind)) = (v["event_id"].as_str(), v["kind"].as_str()) else {
        return Err(AppError::Validation(
            "event_id and kind are required".into(),
        ));
    };
    if event_id.is_empty()
        || event_id.chars().count() > 100
        || kind.is_empty()
        || kind.len() > 60
        || !kind
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
    {
        return Err(AppError::Validation("event_id or kind is not valid".into()));
    }
    let inserted = sqlx::query(
        "INSERT INTO partner_events (event_id, kind, payload) VALUES ($1, $2, $3) \
         ON CONFLICT (event_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(kind)
    .bind(&v)
    .execute(&state.db)
    .await
    .map_err(anyhow::Error::from)?
    .rows_affected();
    // 200 either way: the partner must not retry a settled event.
    Ok((
        StatusCode::OK,
        Json(json!({ "status": if inserted == 1 { "received" } else { "duplicate" } })),
    ))
}
