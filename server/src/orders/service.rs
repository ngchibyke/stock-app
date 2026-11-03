//! Order delivery transitions: plain state changes; the routes check who may
//! make them (the store's owner) and the order's delivery method.
//!
//! - [`mark_picked_up`]: `paid -> picked_up` (sets `picked_up_at`).
//! - [`mark_out_for_delivery`]: `picked_up -> out_for_delivery` (sets
//!   `out_for_delivery_at`; a self-delivery order gets its handover code).
//! - [`mark_delivered`]: `picked_up | out_for_delivery -> delivered` (sets
//!   `delivered_at`, and `release_at = delivered_at + release_after_hours`, the
//!   setting read at that moment, so a later settings change never moves the
//!   deadline). The courier path; a self-delivery seller goes through
//!   [`deliver_with_code`].
//! - [`deliver_with_code`]: `out_for_delivery -> delivered` on a self-delivery
//!   order, only with the buyer's handover code.
//! - [`mark_ready`]: a `paid` courier order is ready for collection (`ready_at`).
//!
//! Each takes the order lock. Repeating a transition on an order already in
//! its target state is a no-op; any other status is 409 `invalid_order_state`.

use anyhow::Context;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    audit, checkout::service::busy_err, crypto::FieldCipher, delivery::handover, error::AppError,
};

const LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";

/// `paid -> picked_up` at `at`.
pub async fn mark_picked_up(
    db: &PgPool,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    transition(db, order_id, at, &["paid"], "picked_up").await
}

/// `picked_up | out_for_delivery -> delivered` at `at`, snapshotting the
/// auto-release deadline.
pub async fn mark_delivered(
    db: &PgPool,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    transition(
        db,
        order_id,
        at,
        &["picked_up", "out_for_delivery"],
        "delivered",
    )
    .await
}

/// `picked_up -> out_for_delivery` at `at`, inside the caller's transaction
/// (`conn`), which this locks the order in. A self-delivery order gets a fresh
/// handover code, encrypted with `cipher`; a courier order never has one.
/// Idempotent: an order already out for delivery keeps its code.
pub async fn mark_out_for_delivery(
    conn: &mut PgConnection,
    cipher: &FieldCipher,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *conn)
        .await
        .context("set lock_timeout")?;
    let (status, self_delivery): (String, bool) = sqlx::query_as(
        "SELECT status::text, COALESCE(delivery_method = 'self', false) \
         FROM orders WHERE id = $1 FOR UPDATE",
    )
    .bind(order_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(busy_err("order_busy", "lock order"))?
    .ok_or(AppError::NotFound)?;
    if status == "out_for_delivery" {
        return Ok(());
    }
    if status != "picked_up" {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    let code = self_delivery.then(|| cipher.encrypt(&handover::generate()));
    sqlx::query(
        "UPDATE orders SET status = 'out_for_delivery', out_for_delivery_at = $2, \
                handover_code_enc = $3, handover_failures = 0, handover_locked_until = NULL \
         WHERE id = $1",
    )
    .bind(order_id)
    .bind(at)
    .bind(code)
    .execute(&mut *conn)
    .await
    .context("mark out for delivery")?;
    audit::record(
        conn,
        None,
        "order.out_for_delivery",
        "order",
        Some(order_id),
        json!({}),
    )
    .await
}

/// What [`deliver_with_code`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handover {
    /// This call moved the order to `delivered`: release it now.
    Delivered,
    /// The order was already `delivered` or `completed`; nothing changed.
    Already,
}

/// `out_for_delivery -> delivered` at `at` on a self-delivery order, only when
/// `code` is the order's handover code (see [`handover::verify`]). A wrong
/// code, or entry while locked, still commits the failure counter and the
/// audit row before the error is returned. On success the code is cleared
/// (single use) and the auto-release deadline is snapshotted, as in
/// [`mark_delivered`]. Releasing the money is the caller's next step, outside
/// this transaction. `actor` is the seller, for the audit.
pub async fn deliver_with_code(
    db: &PgPool,
    cipher: &FieldCipher,
    actor: Uuid,
    order_id: Uuid,
    code: &str,
    at: OffsetDateTime,
) -> Result<Handover, AppError> {
    let mut tx = db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    let (status, self_delivery): (String, bool) = sqlx::query_as(
        "SELECT status::text, COALESCE(delivery_method = 'self', false) \
         FROM orders WHERE id = $1 FOR UPDATE",
    )
    .bind(order_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(busy_err("order_busy", "lock order"))?
    .ok_or(AppError::NotFound)?;
    if !self_delivery {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    match status.as_str() {
        "delivered" | "completed" => return Ok(Handover::Already),
        "out_for_delivery" => {}
        _ => return Err(AppError::Conflict("invalid_order_state")),
    }
    if let Err(e) = handover::verify(&mut tx, cipher, Some(actor), order_id, code).await {
        if matches!(
            e,
            AppError::Unprocessable("handover_code_wrong", _)
                | AppError::ConflictDetail("handover_locked", _)
        ) {
            // The counter, the lock and the audit row must stick.
            tx.commit().await.context("commit")?;
        }
        return Err(e);
    }
    sqlx::query(
        "UPDATE orders SET status = 'delivered', delivered_at = $2, handover_code_enc = NULL, \
                release_at = $2 + make_interval(hours => \
                    (SELECT release_after_hours FROM platform_settings WHERE id)) \
         WHERE id = $1",
    )
    .bind(order_id)
    .bind(at)
    .execute(&mut *tx)
    .await
    .context("mark delivered")?;
    audit::record(
        &mut tx,
        Some(actor),
        "order.delivered",
        "order",
        Some(order_id),
        json!({ "via": "handover_code" }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Handover::Delivered)
}

/// A `paid` courier order is ready for the courier to collect: sets `ready_at`
/// at `at`. Idempotent: once set, `ready_at` keeps its first value (whatever
/// the status by then). Task 9 books the shipment from here.
pub async fn mark_ready(
    db: &PgPool,
    actor: Uuid,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    let (status, courier, ready): (String, bool, bool) = sqlx::query_as(
        "SELECT status::text, COALESCE(delivery_method = 'courier', false), \
                ready_at IS NOT NULL \
         FROM orders WHERE id = $1 FOR UPDATE",
    )
    .bind(order_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(busy_err("order_busy", "lock order"))?
    .ok_or(AppError::NotFound)?;
    if !courier {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    if ready {
        return Ok(());
    }
    if status != "paid" {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    sqlx::query("UPDATE orders SET ready_at = $2 WHERE id = $1")
        .bind(order_id)
        .bind(at)
        .execute(&mut *tx)
        .await
        .context("mark ready")?;
    audit::record(
        &mut tx,
        Some(actor),
        "order.ready",
        "order",
        Some(order_id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(())
}

async fn transition(
    db: &PgPool,
    order_id: Uuid,
    at: OffsetDateTime,
    from: &[&str],
    to: &'static str,
) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    let status: String =
        sqlx::query_scalar("SELECT status::text FROM orders WHERE id = $1 FOR UPDATE")
            .bind(order_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(busy_err("order_busy", "lock order"))?
            .ok_or(AppError::NotFound)?;
    if status == to {
        return Ok(());
    }
    if !from.contains(&status.as_str()) {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    if to == "picked_up" {
        sqlx::query("UPDATE orders SET status = 'picked_up', picked_up_at = $2 WHERE id = $1")
            .bind(order_id)
            .bind(at)
            .execute(&mut *tx)
            .await
            .context("mark picked up")?;
    } else {
        sqlx::query(
            "UPDATE orders SET status = 'delivered', delivered_at = $2, \
                    handover_code_enc = NULL, \
                    release_at = $2 + make_interval(hours => \
                        (SELECT release_after_hours FROM platform_settings WHERE id)) \
             WHERE id = $1",
        )
        .bind(order_id)
        .bind(at)
        .execute(&mut *tx)
        .await
        .context("mark delivered")?;
    }
    audit::record(
        &mut tx,
        None,
        &format!("order.{to}"),
        "order",
        Some(order_id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(())
}
