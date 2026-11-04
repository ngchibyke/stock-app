//! Order delivery transitions: plain state changes; the routes check who may
//! make them (the store's owner) and the order's delivery method.
//!
//! - [`mark_picked_up`]: `paid -> picked_up` (sets `picked_up_at`).
//! - [`mark_out_for_delivery`]: `picked_up -> out_for_delivery` (sets
//!   `out_for_delivery_at`; a self-delivery order gets its handover code).
//! - [`mark_delivered`]: `picked_up | out_for_delivery -> delivered` (sets
//!   `delivered_at`, and `release_at = delivered_at + release_after_hours`, the
//!   setting read at that moment, so a later settings change never moves the
//!   deadline). Not for self-delivery orders: those are 409, whatever their
//!   status, so nothing can skip the handover code.
//! - [`deliver_with_code`]: `out_for_delivery -> delivered` on a self-delivery
//!   order, only with the buyer's handover code.
//! - [`mark_ready`]: a `paid` courier order is ready for collection (`ready_at`),
//!   and gets its `booking_pending` shipment in the same transaction.
//!
//! Each takes the order lock. Repeating a transition on an order already in
//! its target state is a no-op; any other status is 409 `invalid_order_state`.
//!
//! These are the seller's steps: each appends its `delivery_events` row (`source
//! = seller`) in the transition's transaction. A courier update walks the order
//! through the same writes ([`picked_up_in`], [`out_for_delivery_in`],
//! [`delivered_in`]) in its own transaction, with its own event.

use anyhow::Context;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    audit,
    checkout::service::busy_err,
    crypto::FieldCipher,
    delivery::{events, handover, shipments},
    error::AppError,
};

const LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";

/// An order under its lock.
#[derive(sqlx::FromRow)]
struct Locked {
    status: String,
    self_delivery: bool,
    courier: bool,
    ready: bool,
}

/// Takes the order lock in the caller's transaction.
async fn lock_order(conn: &mut PgConnection, order_id: Uuid) -> Result<Locked, AppError> {
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *conn)
        .await
        .context("set lock_timeout")?;
    sqlx::query_as(
        "SELECT status::text AS status, \
                COALESCE(delivery_method = 'self', false) AS self_delivery, \
                COALESCE(delivery_method = 'courier', false) AS courier, \
                ready_at IS NOT NULL AS ready \
         FROM orders WHERE id = $1 FOR UPDATE",
    )
    .bind(order_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(busy_err("order_busy", "lock order"))?
    .ok_or(AppError::NotFound)
}

/// The one "delivered" write: clears any handover code (single use) and
/// snapshots the auto-release deadline from the setting at this moment.
async fn set_delivered(
    conn: &mut PgConnection,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE orders SET status = 'delivered', delivered_at = $2, handover_code_enc = NULL, \
                release_at = $2 + make_interval(hours => \
                    (SELECT release_after_hours FROM platform_settings WHERE id)) \
         WHERE id = $1",
    )
    .bind(order_id)
    .bind(at)
    .execute(conn)
    .await
    .context("mark delivered")?;
    Ok(())
}

/// `paid -> picked_up` at `at`.
pub async fn mark_picked_up(
    db: &PgPool,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    if picked_up_in(&mut tx, order_id, at).await? {
        events::record_seller_step(&mut tx, order_id, "picked_up", at).await?;
    }
    tx.commit().await.context("commit")?;
    Ok(())
}

/// [`mark_picked_up`] with no event, in the caller's transaction. `true` when
/// the order moved.
pub(crate) async fn picked_up_in(
    conn: &mut PgConnection,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<bool, AppError> {
    transition(conn, order_id, at, &["paid"], "picked_up").await
}

/// `picked_up | out_for_delivery -> delivered` at `at`, snapshotting the
/// auto-release deadline. A self-delivery order is 409 `invalid_order_state`:
/// it is only delivered through [`deliver_with_code`].
pub async fn mark_delivered(
    db: &PgPool,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    if delivered_in(&mut tx, order_id, at).await? {
        events::record_seller_step(&mut tx, order_id, "delivered", at).await?;
    }
    tx.commit().await.context("commit")?;
    Ok(())
}

/// [`mark_delivered`] with no event, in the caller's transaction. `true` when
/// the order moved.
pub(crate) async fn delivered_in(
    conn: &mut PgConnection,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<bool, AppError> {
    transition(
        conn,
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
    if out_for_delivery_in(conn, cipher, order_id, at).await? {
        events::record_seller_step(conn, order_id, "out_for_delivery", at).await?;
    }
    Ok(())
}

/// [`mark_out_for_delivery`] with no event. `true` when the order moved.
pub(crate) async fn out_for_delivery_in(
    conn: &mut PgConnection,
    cipher: &FieldCipher,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<bool, AppError> {
    let order = lock_order(conn, order_id).await?;
    if order.status == "out_for_delivery" {
        return Ok(false);
    }
    if order.status != "picked_up" {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    let code = order
        .self_delivery
        .then(|| cipher.encrypt(&handover::generate()));
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
    .await?;
    Ok(true)
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
    let order = lock_order(&mut tx, order_id).await?;
    if !order.self_delivery {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    match order.status.as_str() {
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
    set_delivered(&mut tx, order_id, at).await?;
    audit::record(
        &mut tx,
        Some(actor),
        "order.delivered",
        "order",
        Some(order_id),
        json!({ "via": "handover_code" }),
    )
    .await?;
    events::record_seller_step(&mut tx, order_id, "delivered", at).await?;
    tx.commit().await.context("commit")?;
    Ok(Handover::Delivered)
}

/// A `paid` courier order is ready for the courier to collect: sets `ready_at`
/// at `at` and inserts its `booking_pending` shipment under the order lock, for
/// the job loop to book. Idempotent while the order is still `paid`: `ready_at`
/// keeps its first value and the order keeps its one shipment. Any later status
/// is 409.
pub async fn mark_ready(
    db: &PgPool,
    actor: Uuid,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    let order = lock_order(&mut tx, order_id).await?;
    if !order.courier || order.status != "paid" {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    if !order.ready {
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
        events::record_seller_step(&mut tx, order_id, "ready", at).await?;
    }
    shipments::ensure_pending(&mut tx, order_id).await?;
    tx.commit().await.context("commit")?;
    Ok(())
}

/// `true` when the order moved.
async fn transition(
    conn: &mut PgConnection,
    order_id: Uuid,
    at: OffsetDateTime,
    from: &[&str],
    to: &'static str,
) -> Result<bool, AppError> {
    let order = lock_order(conn, order_id).await?;
    // Guarded here, under the lock: a self-delivery order needs its code.
    if to == "delivered" && order.self_delivery {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    if order.status == to {
        return Ok(false);
    }
    if !from.contains(&order.status.as_str()) {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    if to == "picked_up" {
        sqlx::query("UPDATE orders SET status = 'picked_up', picked_up_at = $2 WHERE id = $1")
            .bind(order_id)
            .bind(at)
            .execute(&mut *conn)
            .await
            .context("mark picked up")?;
    } else {
        set_delivered(conn, order_id, at).await?;
    }
    audit::record(
        conn,
        None,
        &format!("order.{to}"),
        "order",
        Some(order_id),
        json!({}),
    )
    .await?;
    Ok(true)
}
