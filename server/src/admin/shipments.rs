//! Owner re-book and retry-cancel for courier shipments (plan 6, task 7).
//!
//! Booking twice costs real money (see `delivery::shipments`), so a re-book
//! never calls the courier itself: it puts the shipment back to
//! `booking_pending` and the job loop books it through the usual guarded path.
//!
//! - `booking_failed`: the courier refused, or the retries ran out. Allowed
//!   unless the failure was `quote_expired` or `pickup_moved`: a new booking
//!   would replay the same stale token and fail again, and getting a new quote
//!   needs the courier's sandbox answers the README still lists as open (409
//!   `requote_needed`).
//! - `booking_unknown`: the call may have landed. Allowed only when the Owner
//!   confirms, in the request, that they checked the courier's dashboard and no
//!   label exists for this order (`confirm_not_booked`), and the shipment holds
//!   no label of ours (409 `label_exists`).
//! - Anything else is 409 `invalid_shipment_state`; a booking in flight is 409
//!   `booking_in_flight`. The order must still be `paid`.
//!
//! `retry-cancel` puts a `cancel_failed` shipment back in the cancel outbox.
//!
//! Lock order: the order, then the shipment. Both routes are Owner only and
//! audited with a note.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{LOCK_TIMEOUT, Owner, note, parse_id};
use crate::{AppState, audit, checkout::service::busy_err, error::AppError, extract::AppJson};

const NOTE_MAX: usize = 500;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/shipments/{id}/rebook", post(rebook))
        .route("/admin/shipments/{id}/retry-cancel", post(retry_cancel))
}

#[derive(Deserialize)]
struct RebookBody {
    note: Option<String>,
    #[serde(default)]
    confirm_not_booked: bool,
}

#[derive(Deserialize)]
struct RetryBody {
    note: Option<String>,
}

#[derive(sqlx::FromRow)]
struct Locked {
    status: String,
    order_status: String,
    provider_order_id: Option<String>,
    last_error: Option<String>,
}

/// Locks the order, then the shipment (lock order), and reads what the checks need.
async fn lock(tx: &mut PgConnection, id: Uuid) -> Result<Locked, AppError> {
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    // The order id of a shipment never changes, so it may be read before the lock.
    let order_id: Uuid = sqlx::query_scalar("SELECT order_id FROM shipments WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .context("load shipment")?
        .ok_or(AppError::NotFound)?;
    sqlx::query("SELECT id FROM orders WHERE id = $1 FOR UPDATE")
        .bind(order_id)
        .execute(&mut *tx)
        .await
        .map_err(busy_err("order_busy", "lock order"))?;
    sqlx::query_as(
        "SELECT s.status::text AS status, o.status::text AS order_status, \
                s.provider_order_id, s.last_error \
         FROM shipments s JOIN orders o ON o.id = s.order_id \
         WHERE s.id = $1 FOR UPDATE OF s",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(busy_err("order_busy", "lock shipment"))
}

fn wrong_state(status: &str) -> AppError {
    AppError::ConflictDetail("invalid_shipment_state", format!("shipment is {status}"))
}

async fn rebook(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
    AppJson(body): AppJson<RebookBody>,
) -> Result<Json<Value>, AppError> {
    let id = parse_id(&id)?;
    let note = note("note", body.note.as_deref(), NOTE_MAX)?;
    let mut tx = state.db.begin().await.context("begin")?;
    let s = lock(&mut tx, id).await?;
    match s.status.as_str() {
        "booking" => return Err(AppError::Conflict("booking_in_flight")),
        "booking_failed" | "booking_unknown" => {}
        other => return Err(wrong_state(other)),
    }
    if s.provider_order_id.is_some() {
        return Err(AppError::Conflict("label_exists"));
    }
    if s.order_status != "paid" {
        return Err(AppError::ConflictDetail(
            "invalid_order_state",
            format!("order is {}", s.order_status),
        ));
    }
    if s.status == "booking_failed"
        && matches!(
            s.last_error.as_deref(),
            Some("quote_expired" | "pickup_moved")
        )
    {
        return Err(AppError::Conflict("requote_needed"));
    }
    if s.status == "booking_unknown" && !body.confirm_not_booked {
        return Err(AppError::Conflict("confirm_not_booked"));
    }
    sqlx::query(
        "UPDATE shipments SET status = 'booking_pending', attempts = 0, last_error = NULL, \
                sending_until = NULL, next_attempt_at = now(), updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .context("requeue booking")?;
    audit::record(
        &mut tx,
        Some(owner.id),
        "admin.shipment_rebooked",
        "shipment",
        Some(id),
        json!({
            "from": s.status,
            "last_error": s.last_error,
            "confirmed_not_booked": body.confirm_not_booked,
            "note": note,
        }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Json(json!({ "id": id, "status": "booking_pending" })))
}

async fn retry_cancel(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
    AppJson(body): AppJson<RetryBody>,
) -> Result<Json<Value>, AppError> {
    let id = parse_id(&id)?;
    let note = note("note", body.note.as_deref(), NOTE_MAX)?;
    let mut tx = state.db.begin().await.context("begin")?;
    let s = lock(&mut tx, id).await?;
    if s.status != "cancel_failed" {
        return Err(wrong_state(&s.status));
    }
    sqlx::query(
        "UPDATE shipments SET status = 'cancel_pending', cancel_attempts = 0, \
                last_error = NULL, sending_until = NULL, next_attempt_at = now(), \
                updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .context("requeue cancel")?;
    audit::record(
        &mut tx,
        Some(owner.id),
        "admin.shipment_cancel_retried",
        "shipment",
        Some(id),
        json!({ "last_error": s.last_error, "note": note }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Json(json!({ "id": id, "status": "cancel_pending" })))
}
