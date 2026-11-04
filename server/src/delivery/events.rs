//! The append-only `delivery_events` log, and what a courier update does.
//!
//! [`apply_courier_update`] is the one path for webhooks and the poll. Under the
//! order lock, then the shipment lock, it records the event and decides, in one
//! transaction. Replays are neutralised by `dedupe_key` plus the forward-only
//! rank; Shipbubble signs no timestamp, so none is checked.
//!
//! - A rank above the shipment's moves the shipment, and the order with it:
//!   `PickedUp` to `picked_up`, `InTransit` to `out_for_delivery`, `Completed` to
//!   `delivered`. Updates can skip steps, so the order walks through each one it
//!   missed, and always forward: a later status never pulls it back.
//! - Every time on the order is **our receipt time**, never the provider's
//!   `occurred_at`. `Completed` starts the release window, and a back-dated status
//!   cannot shorten it. It never moves money: release is the job loop's, after
//!   the window.
//! - `Cancelled` below `picked_up` sets the shipment `cancelled` with an attention
//!   item, and the order stays `paid` with `ready_at` kept, so the owner can
//!   re-book. On `picked_up` or `in_transit` nothing moves, but a person must look:
//!   the courier disowns a parcel it holds, and only the 30-day backstop would
//!   notice ([`CANCELLED_AFTER_PICKUP`], attention item). After `completed` it is
//!   ignored.
//! - A `cancelled` shipment comes back only on `PickedUp` or later: a parcel moved.
//! - Refunded or cancelled order: the event and the shipment's state are recorded
//!   but the order never moves ([`AFTER_REFUND`], attention item).
//! - `cancel_pending`/`cancel_failed` shipment: `Cancelled` ends it; `PickedUp`
//!   or later means the cancel lost the race ([`CANCEL_LOST`], attention item), and
//!   neither the shipment nor the order moves.
//! - A self-delivery order is never moved: only its handover code delivers it.
//!
//! A signed update for a label no shipment has writes [`UNKNOWN_SHIPMENT`]: the
//! label and the state are the only evidence that can resolve a `booking_unknown`
//! row. An unsigned request never gets this far, so it writes nothing.
//!
//! Ignored updates are logged with the cause only. Neither logs nor audit rows
//! carry the phone, the URL or any provider text. The courier's phone is stored
//! encrypted: a forward move replaces it, an update of the shipment's own rank
//! fills it if we have none (a poll's repeat included, since the phone can appear
//! after the status), and a refunded order keeps none, since nothing will show it.
//! A tracking URL is stored only if it is https.

use anyhow::Context;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{
    AppState, audit,
    checkout::service::busy_err,
    couriers::{CourierError, CourierProvider, CourierUpdate, ShipmentState},
    crypto::FieldCipher,
    error::AppError,
    orders::service,
};

/// Audit action: an update arrived for a refunded or cancelled order.
pub const AFTER_REFUND: &str = "shipment.after_refund";
/// Audit action: the courier moved a parcel whose cancel was asked for.
pub const CANCEL_LOST: &str = "shipment.cancel_lost";
/// Audit action: the courier cancelled a parcel it has collected.
pub const CANCELLED_AFTER_PICKUP: &str = "shipment.cancelled_after_pickup";
/// Audit action: a signed update named a label no shipment has.
pub const UNKNOWN_SHIPMENT: &str = "courier.unknown_shipment";
/// Statuses the poll asks about.
pub const POLLED_STATUSES: &str = "'booked', 'confirmed', 'picked_up', 'in_transit'";
/// A shipment is polled after this long with no event and no poll.
pub const POLL_AFTER: &str = "6 hours";
/// Longest wait for `status`.
const STATUS_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const SUBJECT: &str = "shipment";
const LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";
/// Longest tracking URL kept.
const MAX_TRACKING_URL: usize = 2048;
/// A longer "label" is not one: it is logged, not audited.
const MAX_LABEL_LEN: usize = 200;

/// What [`apply_courier_update`] did.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// The shipment moved, and the order with it where the update calls for it.
    Updated,
    /// Recorded, with an attention item: the order must not move.
    Recorded,
    /// A duplicate, or below the shipment's rank. Nothing moved.
    Ignored,
    /// No shipment has this provider order id. Nothing written.
    UnknownShipment,
}

/// Lock timeouts become 503, so the provider retries.
fn busy(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> AppError {
    move |e| match busy_err("order_busy", ctx)(e) {
        AppError::Conflict(code) => AppError::Unavailable(code),
        other => other,
    }
}

/// A seller's step, in the transition's transaction. One event per order per
/// step: repeating it adds nothing.
pub async fn record_seller_step(
    conn: &mut PgConnection,
    order_id: Uuid,
    status: &str,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO delivery_events (order_id, source, status, occurred_at, dedupe_key) \
         VALUES ($1, 'seller', $2, $3, $4) ON CONFLICT (dedupe_key) DO NOTHING",
    )
    .bind(order_id)
    .bind(status)
    .bind(at)
    .bind(step_key("seller", order_id, status))
    .execute(conn)
    .await
    .context("record seller step")?;
    Ok(())
}

/// The dedupe key of a non-courier event.
fn step_key(source: &str, order_id: Uuid, status: &str) -> String {
    format!("{source}:{order_id}:{status}")
}

/// sha256 of the provider, its order id, the state and the time (the same instant
/// in any offset is the same time). Separated, so that fields cannot run together.
fn courier_key(provider: &str, update: &CourierUpdate) -> Result<String, AppError> {
    let occurred = update
        .occurred_at
        .map(|t| t.to_offset(time::UtcOffset::UTC).format(&Rfc3339))
        .transpose()
        .context("format occurred_at")?
        .unwrap_or_default();
    let mut hash = Sha256::new();
    for part in [
        provider,
        &update.provider_order_id,
        update.state.as_str(),
        &occurred,
    ] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// What an update does to a shipment in `current`.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Above the shipment's rank: move forward to this status.
    Forward(&'static str),
    /// Ends in `cancelled`. `surprise` when we did not ask for it.
    Cancel { surprise: bool },
    /// The courier moved a parcel we asked to cancel.
    CancelLost,
    /// The courier says it cancelled a parcel it holds.
    CancelAfterPickup,
    /// Recorded only, with the cause for the log.
    Ignore(&'static str),
}

/// The shipment's forward rank. `cancelled` is 0, like [`ShipmentState::Cancelled`].
fn rank_of(status: &str) -> Option<u8> {
    Some(match status {
        "cancelled" => 0,
        "booked" => ShipmentState::Pending.rank(),
        "confirmed" => ShipmentState::Confirmed.rank(),
        "picked_up" => ShipmentState::PickedUp.rank(),
        "in_transit" => ShipmentState::InTransit.rank(),
        "completed" => ShipmentState::Completed.rank(),
        _ => return None,
    })
}

fn decide(current: &str, state: ShipmentState) -> Step {
    use ShipmentState::{Cancelled, Completed, Confirmed, InTransit, Pending, PickedUp};
    if matches!(current, "cancel_pending" | "cancel_failed") {
        return match state {
            Cancelled => Step::Cancel { surprise: false },
            PickedUp | InTransit | Completed => Step::CancelLost,
            _ => Step::Ignore("lower_rank"),
        };
    }
    let Some(rank) = rank_of(current) else {
        return Step::Ignore("not_booked");
    };
    match state {
        Cancelled if current == "cancelled" => Step::Ignore("lower_rank"),
        Cancelled if rank < PickedUp.rank() => Step::Cancel { surprise: true },
        Cancelled if current == "completed" => Step::Ignore("cancel_after_completed"),
        Cancelled => Step::CancelAfterPickup,
        Pending | Confirmed if current == "cancelled" => Step::Ignore("lower_rank"),
        s if s.rank() <= rank => Step::Ignore("lower_rank"),
        Confirmed => Step::Forward("confirmed"),
        PickedUp => Step::Forward("picked_up"),
        InTransit => Step::Forward("in_transit"),
        Completed => Step::Forward("completed"),
        // Pending is the lowest rank but `cancelled`, handled above.
        _ => Step::Ignore("lower_rank"),
    }
}

/// Applies one update (see the module docs). Lock busy is 503 `order_busy`, so
/// the provider retries. No provider call happens here.
pub async fn apply_courier_update(
    state: &AppState,
    update: CourierUpdate,
) -> Result<Applied, AppError> {
    let received_at = OffsetDateTime::now_utc();
    let mut tx = state.db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    // Lock order: the order first. A shipment's order never changes, so reading
    // it unlocked is safe.
    let found: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT id, order_id, COALESCE(provider, '') FROM shipments \
         WHERE provider_order_id = $1",
    )
    .bind(&update.provider_order_id)
    .fetch_optional(&mut *tx)
    .await
    .context("find shipment")?;
    let Some((id, order_id, provider)) = found else {
        tracing::warn!(cause = "unknown_shipment", "courier update ignored");
        if update.provider_order_id.len() <= MAX_LABEL_LEN {
            let detail = json!({
                "provider_order_id": update.provider_order_id,
                "state": update.state.as_str(),
            });
            audit::record(&mut tx, None, UNKNOWN_SHIPMENT, "courier", None, detail).await?;
            tx.commit().await.context("commit unknown shipment")?;
        }
        return Ok(Applied::UnknownShipment);
    };
    let (order_status, courier_order): (String, bool) = sqlx::query_as(
        "SELECT status::text, COALESCE(delivery_method = 'courier', false) \
         FROM orders WHERE id = $1 FOR UPDATE",
    )
    .bind(order_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(busy("lock order"))?;
    let shipment_status: String =
        sqlx::query_scalar("SELECT status::text FROM shipments WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(busy("lock shipment"))?;

    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO delivery_events \
             (order_id, source, status, occurred_at, received_at, dedupe_key) \
         VALUES ($1, 'courier', $2, $3, $4, $5) \
         ON CONFLICT (dedupe_key) DO NOTHING RETURNING id",
    )
    .bind(order_id)
    .bind(update.state.as_str())
    .bind(update.occurred_at)
    .bind(received_at)
    .bind(courier_key(&provider, &update)?)
    .fetch_optional(&mut *tx)
    .await
    .context("record courier event")?;
    let refunded = matches!(
        order_status.as_str(),
        "refund_pending" | "refunded" | "cancelled"
    );
    let phone_enc = (!refunded)
        .then_some(update.courier_phone.as_deref())
        .flatten()
        .filter(|p| !p.trim().is_empty())
        .map(|p| state.cipher.encrypt(p));
    // Ahead of the duplicate check: a poll's repeats share a key.
    let same_rank = update.state != ShipmentState::Cancelled
        && rank_of(&shipment_status) == Some(update.state.rank());
    if same_rank && phone_enc.is_some() {
        sqlx::query(
            "UPDATE shipments SET courier_phone_enc = COALESCE(courier_phone_enc, $2) \
             WHERE id = $1",
        )
        .bind(id)
        .bind(&phone_enc)
        .execute(&mut *tx)
        .await
        .context("store courier phone")?;
    }
    if inserted.is_none() {
        tx.commit().await.context("commit repeat")?;
        tracing::info!(cause = "duplicate", "courier update ignored");
        return Ok(Applied::Ignored);
    }

    let step = decide(&shipment_status, update.state);
    let mut change = Change::default();
    let applied = match step {
        Step::Ignore(cause) => {
            tracing::info!(cause, "courier update ignored");
            Applied::Ignored
        }
        Step::Cancel { surprise } => {
            change.status = Some("cancelled");
            change.attention = surprise.then_some("courier_cancelled");
            let reason = if surprise { "courier" } else { "refund" };
            let detail = json!({ "reason": reason, "from": shipment_status });
            audit::record(
                &mut tx,
                None,
                "shipment.cancelled",
                SUBJECT,
                Some(id),
                detail,
            )
            .await?;
            Applied::Updated
        }
        Step::CancelAfterPickup => {
            change.attention = Some("cancelled_after_pickup");
            let detail = json!({ "status": shipment_status });
            audit::record(
                &mut tx,
                None,
                CANCELLED_AFTER_PICKUP,
                SUBJECT,
                Some(id),
                detail,
            )
            .await?;
            tracing::warn!(
                cause = "cancelled_after_pickup",
                "courier update not applied"
            );
            Applied::Recorded
        }
        Step::CancelLost => {
            change.attention = Some("cancel_lost");
            let detail = json!({ "state": update.state.as_str(), "status": shipment_status });
            audit::record(&mut tx, None, CANCEL_LOST, SUBJECT, Some(id), detail).await?;
            if refunded {
                after_refund(&mut tx, id, &update, &order_status).await?;
            }
            Applied::Recorded
        }
        Step::Forward(target) => {
            change.status = Some(target);
            change.phone_enc = phone_enc;
            change.tracking_url = safe_tracking_url(update.tracking_url.as_deref());
            let detail = json!({ "from": shipment_status, "to": target });
            audit::record(
                &mut tx,
                None,
                "shipment.status_changed",
                SUBJECT,
                Some(id),
                detail,
            )
            .await?;
            if refunded {
                change.attention = Some("after_refund");
                after_refund(&mut tx, id, &update, &order_status).await?;
                Applied::Recorded
            } else if courier_order {
                walk(
                    &mut tx,
                    &state.cipher,
                    order_id,
                    &order_status,
                    update.state,
                    received_at,
                )
                .await?;
                Applied::Updated
            } else {
                // `mark_delivered` refuses these: only the handover code delivers.
                change.attention = Some("not_courier_order");
                tracing::warn!(cause = "not_courier_order", "courier update not applied");
                Applied::Recorded
            }
        }
    };
    change.write(&mut tx, id).await?;
    tx.commit().await.context("commit courier update")?;
    Ok(applied)
}

async fn after_refund(
    conn: &mut PgConnection,
    id: Uuid,
    update: &CourierUpdate,
    order_status: &str,
) -> Result<(), AppError> {
    let detail = json!({ "state": update.state.as_str(), "order_status": order_status });
    audit::record(conn, None, AFTER_REFUND, SUBJECT, Some(id), detail).await
}

/// How far along an order is; `None` for a status the courier never moves.
fn progress(status: &str) -> Option<u8> {
    Some(match status {
        "paid" => 0,
        "picked_up" => 1,
        "out_for_delivery" => 2,
        "delivered" | "completed" => 3,
        _ => return None,
    })
}

/// Walks a courier order forward, through every step it missed, to where
/// `state` puts it. The caller holds the order lock and checked the order is a
/// courier order, not refunded.
async fn walk(
    conn: &mut PgConnection,
    cipher: &FieldCipher,
    order_id: Uuid,
    order_status: &str,
    state: ShipmentState,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    let target = match state {
        ShipmentState::PickedUp => 1,
        ShipmentState::InTransit => 2,
        ShipmentState::Completed => 3,
        _ => return Ok(()),
    };
    let Some(now) = progress(order_status) else {
        return Ok(());
    };
    if now < 1 && target >= 1 {
        service::picked_up_in(conn, order_id, at).await?;
    }
    if now < 2 && target >= 2 {
        service::out_for_delivery_in(conn, cipher, order_id, at).await?;
    }
    if now < 3 && target >= 3 {
        service::delivered_in(conn, order_id, at).await?;
    }
    Ok(())
}

/// What an update writes to its shipment.
#[derive(Default)]
struct Change {
    status: Option<&'static str>,
    attention: Option<&'static str>,
    phone_enc: Option<Vec<u8>>,
    tracking_url: Option<String>,
}

impl Change {
    /// An ignored update changes only `last_event_at`.
    async fn write(self, conn: &mut PgConnection, id: Uuid) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE shipments SET status = COALESCE($2::shipment_status, status), \
                    attention = COALESCE($3, attention), \
                    courier_phone_enc = COALESCE($4, courier_phone_enc), \
                    tracking_url = COALESCE($5, tracking_url), \
                    sending_until = CASE WHEN $2 = 'cancelled' THEN NULL \
                                         ELSE sending_until END, \
                    last_event_at = now(), \
                    updated_at = CASE WHEN $2 IS NULL AND $3 IS NULL THEN updated_at \
                                      ELSE now() END \
             WHERE id = $1",
        )
        .bind(id)
        .bind(self.status)
        .bind(self.attention)
        .bind(self.phone_enc)
        .bind(self.tracking_url)
        .execute(conn)
        .await
        .context("update shipment")?;
        Ok(())
    }
}

fn safe_tracking_url(url: Option<&str>) -> Option<String> {
    url.filter(|u| {
        u.len() > "https://".len()
            && u.len() <= MAX_TRACKING_URL
            && u.starts_with("https://")
            && !u.chars().any(|c| c.is_whitespace() || c.is_control())
    })
    .map(str::to_string)
}

/// Asks the provider about one quiet shipment and applies the answer. `status`
/// runs outside any transaction. `Ok(true)` when the answer changed something.
/// A row answered, changed or not, waits [`POLL_AFTER`] again; a failed call
/// takes the jobs' push-back instead.
pub async fn poll_one(
    state: &AppState,
    courier: &dyn CourierProvider,
    id: Uuid,
) -> Result<bool, AppError> {
    let label: Option<String> = sqlx::query_scalar(&format!(
        "SELECT provider_order_id FROM shipments WHERE id = $1 AND status IN ({POLLED_STATUSES})"
    ))
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .context("load shipment")?;
    let Some(label) = label else {
        return Ok(false);
    };
    let mut update = tokio::time::timeout(STATUS_CALL_TIMEOUT, courier.status(&label))
        .await
        .unwrap_or(Err(CourierError::Timeout))?;
    // We asked about this label: the answer can only be about this label.
    update.provider_order_id = label;
    let applied = apply_courier_update(state, update).await?;
    sqlx::query("UPDATE shipments SET last_polled_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await
        .context("mark shipment polled")?;
    Ok(matches!(applied, Applied::Updated | Applied::Recorded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ShipmentState::*;

    #[test]
    fn decide_moves_only_forward() {
        assert_eq!(decide("booked", Confirmed), Step::Forward("confirmed"));
        assert_eq!(decide("booked", Completed), Step::Forward("completed"));
        assert_eq!(decide("picked_up", InTransit), Step::Forward("in_transit"));
        for (current, state) in [
            ("booked", Pending),
            ("confirmed", Confirmed),
            ("picked_up", Confirmed),
            ("completed", PickedUp),
            ("completed", Completed),
        ] {
            assert_eq!(decide(current, state), Step::Ignore("lower_rank"));
        }
    }

    #[test]
    fn decide_cancels_only_before_pickup() {
        let surprise = Step::Cancel { surprise: true };
        assert_eq!(decide("booked", Cancelled), surprise);
        assert_eq!(decide("confirmed", Cancelled), surprise);
        for current in ["picked_up", "in_transit"] {
            assert_eq!(decide(current, Cancelled), Step::CancelAfterPickup);
        }
        assert_eq!(
            decide("completed", Cancelled),
            Step::Ignore("cancel_after_completed")
        );
        assert_eq!(decide("cancelled", Cancelled), Step::Ignore("lower_rank"));
    }

    #[test]
    fn decide_revives_a_cancelled_shipment_only_on_a_moved_parcel() {
        assert_eq!(decide("cancelled", Pending), Step::Ignore("lower_rank"));
        assert_eq!(decide("cancelled", Confirmed), Step::Ignore("lower_rank"));
        assert_eq!(decide("cancelled", PickedUp), Step::Forward("picked_up"));
        assert_eq!(decide("cancelled", Completed), Step::Forward("completed"));
    }

    #[test]
    fn decide_a_cancel_in_progress() {
        for current in ["cancel_pending", "cancel_failed"] {
            assert_eq!(decide(current, Cancelled), Step::Cancel { surprise: false });
            for state in [PickedUp, InTransit, Completed] {
                assert_eq!(decide(current, state), Step::CancelLost);
            }
            for state in [Pending, Confirmed] {
                assert_eq!(decide(current, state), Step::Ignore("lower_rank"));
            }
        }
    }

    #[test]
    fn decide_ignores_shipments_never_booked() {
        for current in [
            "booking_pending",
            "booking",
            "booking_unknown",
            "booking_failed",
        ] {
            assert_eq!(decide(current, Completed), Step::Ignore("not_booked"));
        }
    }

    #[test]
    fn dedupe_key_separates_its_fields_and_ignores_the_offset() {
        let update = |id: &str, at| CourierUpdate {
            provider_order_id: id.into(),
            state: Completed,
            occurred_at: at,
            courier_phone: None,
            tracking_url: None,
        };
        let key = |p, u: &CourierUpdate| courier_key(p, u).unwrap();
        let a = key("ab", &update("c", None));
        assert_eq!(a.len(), 64);
        assert_ne!(a, key("a", &update("bc", None)));
        let utc = time::macros::datetime!(2026-11-01 10:00 UTC);
        let lagos = time::macros::datetime!(2026-11-01 11:00 +1);
        assert_eq!(
            key("p", &update("c", Some(utc))),
            key("p", &update("c", Some(lagos)))
        );
        assert_ne!(
            key("p", &update("c", Some(utc))),
            key("p", &update("c", None))
        );
    }

    #[test]
    fn tracking_url_must_be_https() {
        let ok = "https://track.example/1";
        assert_eq!(safe_tracking_url(Some(ok)).as_deref(), Some(ok));
        for bad in [
            "http://track.example/1",
            "javascript:alert(1)",
            "https://",
            "https://a b",
            "HTTPS-ish",
        ] {
            assert_eq!(safe_tracking_url(Some(bad)), None, "{bad}");
        }
        let long = format!("https://{}", "a".repeat(MAX_TRACKING_URL));
        assert_eq!(safe_tracking_url(Some(&long)), None);
        assert_eq!(safe_tracking_url(None), None);
    }
}
