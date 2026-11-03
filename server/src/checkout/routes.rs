use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::Deserialize;
use uuid::Uuid;

use super::service::{self, CheckoutRequest, CheckoutView, Created, Outcome};
use crate::{
    AppState, auth::AuthUser, delivery::options::DeliveryChoice, error::AppError, extract::AppJson,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/checkout", post(create))
        .route("/checkout/{id}", get(show))
}

const IDEMPOTENCY_HEADER: &str = "idempotency-key";

/// `{method, address_id, deliveries: [{store_id, option_id}]}`: one delivery option
/// per store in the cart, from `POST /checkout/delivery-options`. No amounts: the
/// server prices everything.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckoutBody {
    method: String,
    address_id: Uuid,
    deliveries: Vec<DeliveryChoice>,
}

fn parse_id(key: &str) -> Result<Uuid, AppError> {
    key.parse().map_err(|_| AppError::NotFound)
}

/// The optional `Idempotency-Key`: 1 to 64 of `[A-Za-z0-9_-]` (clients should use a
/// random UUID; never personal data).
fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, AppError> {
    let Some(value) = headers.get(IDEMPOTENCY_HEADER) else {
        return Ok(None);
    };
    let invalid = || {
        AppError::Validation("Idempotency-Key must be 1 to 64 letters, digits, '-' or '_'".into())
    };
    let key = value.to_str().map_err(|_| invalid())?;
    if key.is_empty()
        || key.len() > 64
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(invalid());
    }
    Ok(Some(key.to_string()))
}

async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    AppJson(body): AppJson<CheckoutBody>,
) -> Result<(StatusCode, Json<Created>), AppError> {
    let key = idempotency_key(&headers)?;
    let request = CheckoutRequest {
        method: body.method,
        address_id: body.address_id,
        deliveries: body.deliveries,
    };
    match service::create(&state, user.id, &request, key.as_deref()).await? {
        Outcome::Created(c) => Ok((StatusCode::CREATED, Json(c))),
        // A replay answers exactly like the original request.
        Outcome::Replayed(c) => Ok((StatusCode::CREATED, Json(c))),
    }
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<CheckoutView>, AppError> {
    let id = parse_id(&id)?;
    // Fallback for a lost webhook: a pending checkout is verified with the
    // provider (rate-limited by the verify gate) before it is shown.
    super::settle::settle_for_buyer(&state, user.id, id).await;
    Ok(Json(service::get(&state.db, user.id, id).await?))
}
