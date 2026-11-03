//! Courier shipments: one row per courier order, created when the seller marks
//! it ready. The job loop books and cancels them (`escrow/jobs.rs` claims, this
//! module acts).
//!
//! **Booking twice costs real money.** `book` debits the platform's prepaid
//! wallet and the provider has no idempotency key, so the row is the only guard:
//!
//! 1. A short transaction, under the order and shipment locks, re-checks the row
//!    and takes a [`LEASE`] lease in `booking`. A pickup that really moved since
//!    the quote (`stores.pickup_moved_at`), or a quote [`QUOTE_VALID_DAYS`] old,
//!    ends in `booking_failed` here, and `book` is never called. A changed sender
//!    contact alone still books, with `sender_stale` in the audit row: the store's
//!    address code is also cleared by a phone change and by resaving the same
//!    address, so it cannot say whether the courier would go to the wrong place.
//! 2. `book` runs outside any transaction, cut off after [`BOOK_CALL_TIMEOUT`]
//!    (well under the lease, so a live worker always answers within it).
//! 3. The outcome is written under the order lock, fenced on the lease
//!    (`status = 'booking' AND sending_until = <ours>`):
//!    - success: `booked`, with what the provider charged. A charge above the
//!      quote plus the tolerance raises `shipment.price_above_quote`;
//!    - `Unavailable` (definitely not sent): back to `booking_pending` with the
//!      jobs' backoff, then `booking_failed` after [`MAX_BOOK_ATTEMPTS`] calls;
//!    - `Rejected`: `booking_failed`, with `last_error` the code only;
//!    - anything else: `booking_unknown`, never retried automatically.
//!
//! If the fence fails the row is not touched: the answer is logged without
//! provider text, and audited ([`LATE_OUTCOME`]) so the attention list can show
//! a label it created. If the write itself fails after a successful POST, the
//! label's id is logged, since nothing else keeps it.
//!
//! **A row left in `booking` past its lease** goes to `booking_unknown`
//! ([`reap_stale_leases`]), never back to `booking_pending`: the worker died
//! somewhere around the POST, and it may have landed.
//!
//! **Refunds** ([`on_refund`], inside the refund's transaction, under the order
//! lock) never depend on the courier:
//! - `booked`/`confirmed` -> `cancel_pending`, an outbox the job loop drains
//!   ([`cancel_one`]) after commit: `cancelled`, or `cancel_failed` with an audit
//!   row (a cancel that never left retries first). A crash after the commit
//!   cannot lose the cancel.
//! - `booking_pending` -> `cancelled`, with no provider call.
//! - `booking`/`booking_unknown`: left alone, with an audit row, and listed for
//!   the owner: a shipment may be live on a refunded order. If a booking in flight
//!   then lands, its outcome write sees the refund and goes to `cancel_pending`.
//!
//! Lock order: orders, then shipments. Data protection: the courier's phone is
//! stored only encrypted with `FieldCipher`, and neither logs nor audit rows
//! carry it. `last_error` holds our own short codes, never provider text.

use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState, audit,
    checkout::service::busy_err,
    couriers::{Booking, CourierError, CourierProvider},
    error::AppError,
    escrow::jobs::BACKOFF_SQL,
};

/// Calls to `book` before an always-unavailable provider gives up.
pub const MAX_BOOK_ATTEMPTS: i32 = 8;
/// Calls to `cancel` before an always-unavailable provider gives up.
pub const MAX_CANCEL_ATTEMPTS: i32 = 8;
/// A request token is valid this long from the quote.
pub const QUOTE_VALID_DAYS: i32 = 7;
/// How long a booking or cancel call holds its row.
pub const LEASE: &str = "2 minutes";
/// Longest wait for `book`; past it the outcome is a Timeout.
pub const BOOK_CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest wait for `cancel`.
pub const CANCEL_CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Audit action: the provider charged more than the quote plus the tolerance.
pub const PRICE_ABOVE_QUOTE: &str = "shipment.price_above_quote";
/// Audit action: a booking answer arrived after its lease was lost.
pub const LATE_OUTCOME: &str = "shipment.late_outcome";
/// Audit action behind the reconciliation's `wallet_alerts`.
pub const WALLET_ALERT: &str = "courier.wallet_alert";
/// A booking 400 may be an empty wallet; the two cannot be told apart.
const WALLET_REFUSAL_CODE: &str = "http_400";
const SUBJECT: &str = "shipment";
const LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";

/// Inserts the order's `booking_pending` shipment, unless it has one. The
/// caller holds the order lock.
pub async fn ensure_pending(conn: &mut PgConnection, order_id: Uuid) -> Result<(), AppError> {
    let id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO shipments (order_id, quoted_kobo) \
         SELECT id, courier_quote_kobo FROM orders WHERE id = $1 \
         ON CONFLICT (order_id) DO NOTHING RETURNING id",
    )
    .bind(order_id)
    .fetch_optional(&mut *conn)
    .await
    .context("insert shipment")?;
    if let Some(id) = id {
        audit::record(
            conn,
            None,
            "shipment.created",
            SUBJECT,
            Some(id),
            json!({ "order_id": order_id }),
        )
        .await?;
    }
    Ok(())
}

/// Ready courier orders still `paid` with no shipment get one.
pub async fn create_missing(db: &PgPool, limit: i64) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT o.id FROM orders o \
         WHERE o.delivery_method = 'courier' AND o.ready_at IS NOT NULL AND o.status = 'paid' \
           AND NOT EXISTS (SELECT 1 FROM shipments s WHERE s.order_id = o.id) \
         ORDER BY o.ready_at, o.id LIMIT $1 FOR UPDATE OF o SKIP LOCKED",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .context("claim ready orders")?;
    for id in ids {
        ensure_pending(&mut tx, id).await?;
    }
    tx.commit().await.context("commit shipments")?;
    Ok(())
}

/// `booking` rows past their lease go to `booking_unknown` (see the module docs).
pub async fn reap_stale_leases(db: &PgPool, limit: i64) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "WITH stale AS ( \
             SELECT id FROM shipments WHERE status = 'booking' AND sending_until < now() \
             ORDER BY sending_until, id LIMIT $1 FOR UPDATE SKIP LOCKED) \
         UPDATE shipments s SET status = 'booking_unknown', last_error = 'lease_expired', \
                sending_until = NULL, updated_at = now() \
         FROM stale WHERE s.id = stale.id RETURNING s.id",
    )
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .context("reap booking leases")?;
    for id in &ids {
        audit::record(
            &mut tx,
            None,
            "shipment.booking_unknown",
            SUBJECT,
            Some(*id),
            json!({ "reason": "lease_expired" }),
        )
        .await?;
    }
    tx.commit().await.context("commit reaped leases")?;
    if !ids.is_empty() {
        tracing::warn!(
            count = ids.len(),
            "bookings left past their lease; moved to booking_unknown"
        );
    }
    Ok(())
}

/// What a refund does to the order's shipment (see the module docs). Runs in the
/// refund's transaction, which already holds the order lock.
pub async fn on_refund(conn: &mut PgConnection, order_id: Uuid) -> Result<(), AppError> {
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, status::text FROM shipments WHERE order_id = $1 FOR UPDATE")
            .bind(order_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(busy_err("order_busy", "lock shipment"))?;
    let Some((id, status)) = row else {
        return Ok(());
    };
    let (target, action, detail) = match status.as_str() {
        "booked" | "confirmed" => (
            Some("cancel_pending"),
            "shipment.cancel_requested",
            json!({ "reason": "refund", "from": status }),
        ),
        "booking_pending" => (
            Some("cancelled"),
            "shipment.cancelled",
            json!({ "reason": "refund_before_booking" }),
        ),
        "booking" | "booking_unknown" => (
            None,
            "shipment.live_on_refund",
            json!({ "order_id": order_id, "status": status }),
        ),
        _ => return Ok(()),
    };
    if let Some(target) = target {
        sqlx::query(
            "UPDATE shipments SET status = $2::shipment_status, sending_until = NULL, \
                    next_attempt_at = now(), updated_at = now(), \
                    cancel_requested_at = CASE WHEN $2::shipment_status = 'cancel_pending' \
                                               THEN now() END \
             WHERE id = $1",
        )
        .bind(id)
        .bind(target)
        .execute(&mut *conn)
        .await
        .context("move shipment on refund")?;
    }
    audit::record(conn, None, action, SUBJECT, Some(id), detail).await
}

/// A leased shipment, ready to book. Holds the request token: no `Debug`.
struct BookingCall {
    id: Uuid,
    order_id: Uuid,
    lease: OffsetDateTime,
    attempts: i32,
    quoted_kobo: i64,
    request_token: String,
    courier_id: String,
    service_code: String,
}

#[derive(sqlx::FromRow)]
struct DueCheck {
    due: bool,
    pickup_moved: bool,
    sender_stale: bool,
    quote_expired: bool,
    quoted_kobo: i64,
    request_token: String,
    courier_id: String,
    service_code: String,
}

/// Books one claimed shipment. `Ok(true)` when the provider booked it and the
/// outcome is on the row; `Ok(false)` otherwise. Never call it inside an open
/// transaction. An error after the call leaves the row in `booking`, which the
/// reaper takes to `booking_unknown`.
pub async fn book_one(
    state: &AppState,
    courier: &dyn CourierProvider,
    id: Uuid,
    tolerance_bps: i32,
) -> Result<bool, AppError> {
    let Some(call) = take_lease(&state.db, courier.name(), id).await? else {
        return Ok(false);
    };
    let result = tokio::time::timeout(
        BOOK_CALL_TIMEOUT,
        courier.book(&call.request_token, &call.courier_id, &call.service_code),
    )
    .await
    .unwrap_or(Err(CourierError::Timeout));
    let label = result.as_ref().ok().map(|b| b.provider_order_id.clone());
    let recorded = record_booking(state, &call, result, tolerance_bps).await;
    if let (Err(_), Some(label)) = (&recorded, &label) {
        // The provider's order id is not personal data, and only this line keeps it.
        tracing::error!(
            shipment_id = %id,
            provider_order_id = %label,
            "booking made but its outcome was not recorded"
        );
    }
    recorded
}

/// Step 1 of the module docs. `None` when there is nothing to send: the row is
/// no longer due, or it just failed its pre-booking checks.
async fn take_lease(
    db: &PgPool,
    provider: &str,
    id: Uuid,
) -> Result<Option<BookingCall>, AppError> {
    let mut tx = db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    // Lock order: the order first. Its id never changes, so reading it
    // unlocked is safe.
    let order_id: Uuid = sqlx::query_scalar("SELECT order_id FROM shipments WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .context("load shipment")?;
    sqlx::query("SELECT id FROM orders WHERE id = $1 FOR UPDATE")
        .bind(order_id)
        .execute(&mut *tx)
        .await
        .map_err(busy_err("order_busy", "lock order"))?;
    let check: DueCheck = sqlx::query_as(
        "SELECT s.status = 'booking_pending' AND s.next_attempt_at <= now() \
                    AND o.status = 'paid' AND o.held_at IS NULL \
                    AND NOT c.provider_flagged AS due, \
                COALESCE(st.pickup_moved_at >= COALESCE(d.created_at, o.created_at), false) \
                    AS pickup_moved, \
                st.pickup_courier_address_code IS DISTINCT FROM o.courier_sender_code \
                    AS sender_stale, \
                COALESCE(d.created_at, o.created_at) <= now() - make_interval(days => $2) \
                    AS quote_expired, \
                s.quoted_kobo, o.courier_request_token AS request_token, o.courier_id, \
                o.service_code \
         FROM shipments s JOIN orders o ON o.id = s.order_id \
              JOIN checkouts c ON c.id = o.checkout_id JOIN stores st ON st.id = o.store_id \
              LEFT JOIN delivery_options d ON d.id = o.delivery_option_id \
         WHERE s.id = $1 FOR UPDATE OF s",
    )
    .bind(id)
    .bind(QUOTE_VALID_DAYS)
    .fetch_one(&mut *tx)
    .await
    .map_err(busy_err("order_busy", "lock shipment"))?;
    if !check.due {
        return Ok(None);
    }
    let refused = if check.pickup_moved {
        Some("pickup_moved")
    } else if check.quote_expired {
        Some("quote_expired")
    } else {
        None
    };
    if let Some(reason) = refused {
        sqlx::query(
            "UPDATE shipments SET status = 'booking_failed', last_error = $2, \
                    updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(reason)
        .execute(&mut *tx)
        .await
        .context("refuse booking")?;
        audit::record(
            &mut tx,
            None,
            "shipment.booking_failed",
            SUBJECT,
            Some(id),
            json!({ "reason": reason }),
        )
        .await?;
        tx.commit().await.context("commit refused booking")?;
        return Ok(None);
    }
    let (lease, attempts): (OffsetDateTime, i32) = sqlx::query_as(&format!(
        "UPDATE shipments SET status = 'booking', provider = $2, attempts = attempts + 1, \
                sending_until = now() + interval '{LEASE}', updated_at = now() \
         WHERE id = $1 RETURNING sending_until, attempts"
    ))
    .bind(id)
    .bind(provider)
    .fetch_one(&mut *tx)
    .await
    .context("take booking lease")?;
    audit::record(
        &mut tx,
        None,
        "shipment.booking_started",
        SUBJECT,
        Some(id),
        json!({ "attempt": attempts, "sender_stale": check.sender_stale }),
    )
    .await?;
    tx.commit().await.context("commit booking lease")?;
    Ok(Some(BookingCall {
        id,
        order_id,
        lease,
        attempts,
        quoted_kobo: check.quoted_kobo,
        request_token: check.request_token,
        courier_id: check.courier_id,
        service_code: check.service_code,
    }))
}

/// What a booking answer means for the row.
struct Outcome {
    status: &'static str,
    error: Option<String>,
    booking: Option<Booking>,
    audit: (&'static str, Value),
}

/// `refunded`: the order was refunded while the call was in flight, so a label
/// that was made must go, and one that was not needs nobody.
fn decide(result: Result<Booking, CourierError>, attempts: i32, refunded: bool) -> Outcome {
    let stop = |status, error: String, action, detail| Outcome {
        status,
        error: Some(error),
        booking: None,
        audit: (action, detail),
    };
    let cancelled = |error: String| {
        stop(
            "cancelled",
            error,
            "shipment.cancelled",
            json!({ "reason": "refund_during_booking" }),
        )
    };
    let failed = |error: String| {
        let detail = json!({ "reason": error });
        stop("booking_failed", error, "shipment.booking_failed", detail)
    };
    let unknown = |error: &str| {
        stop(
            "booking_unknown",
            error.to_string(),
            "shipment.booking_unknown",
            json!({ "reason": error }),
        )
    };
    match result {
        Ok(booking) => Outcome {
            status: if refunded { "cancel_pending" } else { "booked" },
            error: None,
            audit: (
                "shipment.booked",
                json!({
                    "provider_order_id": booking.provider_order_id,
                    "charged_kobo": booking.charged_kobo,
                }),
            ),
            booking: Some(booking),
        },
        Err(CourierError::Unavailable) if refunded => cancelled("unavailable".into()),
        Err(CourierError::Rejected { code, .. }) if refunded => cancelled(code),
        Err(CourierError::Unavailable) if attempts >= MAX_BOOK_ATTEMPTS => {
            failed("unavailable".into())
        }
        Err(CourierError::Unavailable) => stop(
            "booking_pending",
            "unavailable".into(),
            "shipment.booking_retry",
            json!({ "attempts": attempts }),
        ),
        Err(CourierError::Rejected { code, .. }) => failed(code),
        Err(CourierError::Timeout) => unknown("timed out"),
        Err(CourierError::Inconclusive) => unknown("inconclusive"),
        // Not a booking answer at all: nothing proves the POST did not land.
        Err(_) => unknown("unexpected"),
    }
}

/// Above `quoted × (1 + bps / 10,000)`, compared exactly (no rounding).
fn above_tolerance(quoted: i64, charged: i64, bps: i32) -> bool {
    i128::from(charged) * 10_000 > i128::from(quoted) * (10_000 + i128::from(bps))
}

/// Step 3 of the module docs.
async fn record_booking(
    state: &AppState,
    call: &BookingCall,
    result: Result<Booking, CourierError>,
    tolerance_bps: i32,
) -> Result<bool, AppError> {
    let wallet_refusal = matches!(
        &result,
        Err(CourierError::Rejected { code, .. }) if code == WALLET_REFUSAL_CODE
    );
    // No lock_timeout here: this answer may have cost money, so it waits its turn.
    let mut tx = state.db.begin().await.context("begin")?;
    let order_status: String =
        sqlx::query_scalar("SELECT status::text FROM orders WHERE id = $1 FOR UPDATE")
            .bind(call.order_id)
            .fetch_one(&mut *tx)
            .await
            .context("lock order")?;
    let refunded = matches!(
        order_status.as_str(),
        "refund_pending" | "refunded" | "cancelled"
    );
    let outcome = decide(result, call.attempts, refunded);
    let booking = outcome.booking.as_ref();
    let phone_enc = booking
        .and_then(|b| b.courier_phone.as_deref())
        .map(|phone| state.cipher.encrypt(phone));
    let charged = booking.and_then(|b| b.charged_kobo);
    let fenced: Option<Uuid> = sqlx::query_scalar(&format!(
        "UPDATE shipments SET status = $3::shipment_status, provider_order_id = $4, \
                tracking_url = $5, courier_name = $6, courier_phone_enc = $7, \
                charged_kobo = $8, last_error = $9, sending_until = NULL, updated_at = now(), \
                next_attempt_at = CASE WHEN $3::shipment_status = 'booking_pending' \
                    THEN now() + make_interval(secs => \
                        (SELECT {BACKOFF_SQL} FROM (SELECT $10::int AS a) x)) \
                    ELSE now() END, \
                cancel_requested_at = CASE WHEN $3::shipment_status = 'cancel_pending' \
                                           THEN now() END \
         WHERE id = $1 AND status = 'booking' AND sending_until = $2 RETURNING id"
    ))
    .bind(call.id)
    .bind(call.lease)
    .bind(outcome.status)
    .bind(booking.map(|b| b.provider_order_id.as_str()))
    .bind(booking.and_then(|b| b.tracking_url.as_deref()))
    .bind(booking.map(|b| b.courier_name.as_str()))
    .bind(&phone_enc)
    .bind(charged)
    .bind(&outcome.error)
    .bind(call.attempts)
    .fetch_optional(&mut *tx)
    .await
    .context("record booking outcome")?;
    if fenced.is_none() {
        tracing::warn!(
            outcome = outcome.status,
            "booking lease lost; late answer not recorded"
        );
        audit::record(
            &mut tx,
            None,
            LATE_OUTCOME,
            SUBJECT,
            Some(call.id),
            json!({
                "outcome": outcome.status,
                "provider_order_id": booking.map(|b| b.provider_order_id.as_str()),
                "charged_kobo": charged,
            }),
        )
        .await?;
        tx.commit().await.context("commit late outcome")?;
        return Ok(false);
    }
    let (action, detail) = &outcome.audit;
    audit::record(
        &mut tx,
        None,
        action,
        SUBJECT,
        Some(call.id),
        detail.clone(),
    )
    .await?;
    if outcome.status == "cancel_pending" {
        audit::record(
            &mut tx,
            None,
            "shipment.cancel_requested",
            SUBJECT,
            Some(call.id),
            json!({ "reason": "refund_during_booking", "from": "booking" }),
        )
        .await?;
    }
    if let Some(charged) = charged
        && above_tolerance(call.quoted_kobo, charged, tolerance_bps)
    {
        audit::record(
            &mut tx,
            None,
            PRICE_ABOVE_QUOTE,
            SUBJECT,
            Some(call.id),
            json!({ "quoted": call.quoted_kobo, "charged": charged }),
        )
        .await?;
    }
    if wallet_refusal {
        audit::record(
            &mut tx,
            None,
            WALLET_ALERT,
            SUBJECT,
            Some(call.id),
            json!({ "event": "possible_low_balance" }),
        )
        .await?;
    }
    tx.commit().await.context("commit booking outcome")?;
    tracing::info!(outcome = outcome.status, "booking answered");
    Ok(booking.is_some())
}

/// Drains one `cancel_pending` row: a lease, `cancel` outside any transaction,
/// then the outcome fenced on the lease. `Ok(true)` when it ended `cancelled`.
/// `Unavailable` means the call never left, so the row stays `cancel_pending`
/// with the jobs' backoff, up to [`MAX_CANCEL_ATTEMPTS`] calls. Every other
/// error is `cancel_failed`, for the owner. A worker that dies mid-call leaves
/// `cancel_pending`, and the row is sent again once its lease runs out; a
/// cancel repeated that way can only fail, which flags the row.
pub async fn cancel_one(
    state: &AppState,
    courier: &dyn CourierProvider,
    id: Uuid,
) -> Result<bool, AppError> {
    let claimed: Option<(OffsetDateTime, i32, String)> = sqlx::query_as(&format!(
        "UPDATE shipments SET sending_until = now() + interval '{LEASE}', \
                cancel_attempts = cancel_attempts + 1, updated_at = now() \
         WHERE id = $1 AND status = 'cancel_pending' AND next_attempt_at <= now() \
           AND (sending_until IS NULL OR sending_until < now()) \
         RETURNING sending_until, cancel_attempts, provider_order_id"
    ))
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .context("claim cancel")?;
    let Some((lease, attempts, provider_order_id)) = claimed else {
        return Ok(false);
    };
    let result = tokio::time::timeout(CANCEL_CALL_TIMEOUT, courier.cancel(&provider_order_id))
        .await
        .unwrap_or(Err(CourierError::Timeout));
    let (status, action, error): (&str, &str, Option<String>) = match result {
        Ok(()) => ("cancelled", "shipment.cancelled", None),
        Err(CourierError::Unavailable) if attempts < MAX_CANCEL_ATTEMPTS => (
            "cancel_pending",
            "shipment.cancel_retry",
            Some("unavailable".into()),
        ),
        Err(e) => {
            let error = match e {
                CourierError::Rejected { code, .. } => code,
                CourierError::Unavailable => "unavailable".into(),
                CourierError::Timeout => "timed out".into(),
                CourierError::Inconclusive => "inconclusive".into(),
                _ => "unexpected".into(),
            };
            ("cancel_failed", "shipment.cancel_failed", Some(error))
        }
    };
    let mut tx = state.db.begin().await.context("begin")?;
    let fenced: Option<Uuid> = sqlx::query_scalar(&format!(
        "UPDATE shipments SET status = $3::shipment_status, last_error = $4, \
                sending_until = NULL, updated_at = now(), \
                next_attempt_at = now() + make_interval(secs => \
                    (SELECT {BACKOFF_SQL} FROM (SELECT $5::int AS a) x)) \
         WHERE id = $1 AND status = 'cancel_pending' AND sending_until = $2 RETURNING id"
    ))
    .bind(id)
    .bind(lease)
    .bind(status)
    .bind(&error)
    .bind(attempts)
    .fetch_optional(&mut *tx)
    .await
    .context("record cancel outcome")?;
    if fenced.is_none() {
        tracing::warn!(
            outcome = status,
            "cancel lease lost; late answer not recorded"
        );
        audit::record(
            &mut tx,
            None,
            "shipment.late_cancel_outcome",
            SUBJECT,
            Some(id),
            json!({ "outcome": status, "error": error }),
        )
        .await?;
        tx.commit().await.context("commit late cancel")?;
        return Ok(false);
    }
    audit::record(
        &mut tx,
        None,
        action,
        SUBJECT,
        Some(id),
        json!({ "reason": "refund", "error": error, "attempt": attempts }),
    )
    .await?;
    tx.commit().await.context("commit cancel outcome")?;
    Ok(status == "cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerance_is_exact() {
        assert!(!above_tolerance(150_000, 165_000, 1000));
        assert!(above_tolerance(150_000, 165_001, 1000));
        assert!(!above_tolerance(150_000, 150_000, 0));
        assert!(above_tolerance(150_000, 150_001, 0));
        // A charge on a free quote is above any tolerance.
        assert!(above_tolerance(0, 1, 5000));
        assert!(!above_tolerance(i64::MAX, i64::MAX, 0));
    }
}
