//! Collect from a branch (part 7e-1, task 10).
//!
//! A buyer can pick an order up at the branch that holds the goods instead of
//! having it delivered (`delivery_method = 'collect'`, fee 0, offered where the
//! branch has `collect_enabled`; see [`super::options`]). The steps:
//!
//! - **Ready** ([`mark_ready`]): a member who may act on the order marks a `paid`
//!   collect order ready at the counter. That sets `ready_at`, the deadline
//!   `collect_by = ready_at + collect_window_days` (read now, so a later settings
//!   change never moves it), and the buyer's handover code (6 digits from the OS
//!   CSPRNG, FieldCipher-encrypted), and tells the buyer
//!   (`order_ready_for_collection`). Repeating it changes nothing.
//! - **Collected** ([`collect_with_code`]): the buyer shows the code at the
//!   counter and a member enters it. [`handover::verify`] checks it with the same
//!   lockout as a self-delivery handover. The order moves to `delivered`, the code
//!   is cleared, and the existing release clock starts (`release_at`); the money
//!   is not released at once (the buyer can still confirm early or report a
//!   problem). Audited `order.collected`.
//! - **Reminders** ([`send_reminders`], job `collect_reminders`): the buyer gets
//!   `collect_reminder` 1 day and 4 days after ready, each at most once (dedupe
//!   keys `collect_reminder:{order}:1` and `:4`), never late (day 1 is skipped once
//!   day 4 is due) and never once `collect_by` has passed.
//! - **Not collected** ([`expire_one`], job `expire_collections`): an order still
//!   `paid` at `collect_by`, not held and its payment not flagged, is refunded in
//!   full with its stock back at its branch (through [`begin_refund`], the one
//!   refund path, which uses `stock::apply`), with **no** `seller_strikes` row: the
//!   seller did their part. The buyer and the team (`Audience::Order`) are told
//!   (`collection_expired`); audited `order.collection_expired`.
//!
//! **Late collection against expiry.** Both take the order row lock and re-check
//! the status under it: collection needs `paid`, the refund needs `paid`. So
//! exactly one wins; a collected order is never refunded, a refunded one is never
//! collected (its code is cleared by the refund, too).
//!
//! **Personal data.** Audit rows carry ids only, never the code. The buyer sees the
//! branch's address and phone only after paying (`delivery::tracking`); the branch
//! sees the buyer's display name, never a phone, and a collect order keeps no copy
//! of the buyer's address.

use anyhow::Context;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{events, handover};
use crate::{
    AppState, audit,
    checkout::service::busy_err,
    crypto::FieldCipher,
    error::AppError,
    escrow::{
        jobs::{BATCH, Stats},
        refund::{begin_refund, send_refund},
    },
    notify::{
        kinds::Kind,
        service::{self as notify, Audience, Notice, enqueue},
    },
    orders::service::{Handover, set_delivered},
};

/// Job names (`job_runs.job`).
pub const EXPIRE_COLLECTIONS: &str = "expire_collections";
pub const COLLECT_REMINDERS: &str = "collect_reminders";
/// Audit action and refund reason of an uncollected order.
pub const COLLECTION_EXPIRED: &str = "order.collection_expired";
pub const REFUND_REASON: &str = "collection_expired";
/// Days after ready that the buyer is reminded.
pub const REMINDER_DAYS: [i32; 2] = [1, 4];

const LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";

/// SQL over `orders o` joined to `checkouts c`: a ready collect order past its
/// `collect_by`, not held, its payment not flagged. The claim and the re-check
/// under the lock both use it, so they cannot drift apart.
pub(crate) const EXPIRY_SCOPE: &str = "o.delivery_method = 'collect' \
     AND o.ready_at IS NOT NULL AND o.collect_by IS NOT NULL AND o.collect_by <= now() \
     AND o.held_at IS NULL AND NOT c.provider_flagged";

/// A collect order under its lock.
struct Locked {
    status: String,
    collect: bool,
    ready: bool,
}

async fn lock_order(conn: &mut PgConnection, order_id: Uuid) -> Result<Locked, AppError> {
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *conn)
        .await
        .context("set lock_timeout")?;
    let (status, collect, ready): (String, bool, bool) = sqlx::query_as(
        "SELECT status::text, COALESCE(delivery_method = 'collect', false), \
                ready_at IS NOT NULL \
         FROM orders WHERE id = $1 FOR UPDATE",
    )
    .bind(order_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(busy_err("order_busy", "lock order"))?
    .ok_or(AppError::NotFound)?;
    Ok(Locked {
        status,
        collect,
        ready,
    })
}

/// A `paid` collect order is ready at the counter (see the module docs). Any other
/// method or status is 409 `invalid_order_state`. `actor` is the member, for the
/// audit.
pub async fn mark_ready(
    db: &PgPool,
    cipher: &FieldCipher,
    actor: Uuid,
    order_id: Uuid,
    at: OffsetDateTime,
) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    let order = lock_order(&mut tx, order_id).await?;
    if !order.collect || order.status != "paid" {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    if order.ready {
        return Ok(());
    }
    let code = cipher.encrypt(&handover::generate());
    sqlx::query(
        "UPDATE orders SET ready_at = $2, \
                collect_by = $2 + make_interval(days => \
                    (SELECT collect_window_days FROM platform_settings WHERE id)), \
                handover_code_enc = $3, handover_failures = 0, handover_locked_until = NULL \
         WHERE id = $1",
    )
    .bind(order_id)
    .bind(at)
    .bind(code)
    .execute(&mut *tx)
    .await
    .context("mark ready for collection")?;
    audit::record(
        &mut tx,
        Some(actor),
        "order.ready",
        "order",
        Some(order_id),
        json!({ "method": "collect" }),
    )
    .await?;
    events::record_seller_step(&mut tx, order_id, "ready", at).await?;
    notify::tell_buyer(&mut tx, Kind::OrderReadyForCollection, order_id).await?;
    tx.commit().await.context("commit")?;
    Ok(())
}

/// A ready collect order handed over against the buyer's code (see the module
/// docs). A wrong code, or entry while locked, still commits the failure counter
/// and the audit row before the error is returned. An order already `delivered` or
/// `completed` is [`Handover::Already`]; any other state is 409
/// `invalid_order_state`.
pub async fn collect_with_code(
    db: &PgPool,
    cipher: &FieldCipher,
    actor: Uuid,
    order_id: Uuid,
    code: &str,
    at: OffsetDateTime,
) -> Result<Handover, AppError> {
    let mut tx = db.begin().await.context("begin")?;
    let order = lock_order(&mut tx, order_id).await?;
    if !order.collect {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    match order.status.as_str() {
        "delivered" | "completed" => return Ok(Handover::Already),
        "paid" if order.ready => {}
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
    sqlx::query("UPDATE orders SET collect_by = NULL WHERE id = $1")
        .bind(order_id)
        .execute(&mut *tx)
        .await
        .context("clear collect deadline")?;
    audit::record(
        &mut tx,
        Some(actor),
        "order.collected",
        "order",
        Some(order_id),
        json!({ "via": "handover_code" }),
    )
    .await?;
    events::record_seller_step(&mut tx, order_id, "delivered", at).await?;
    notify::tell_buyer(&mut tx, Kind::OrderDelivered, order_id).await?;
    tx.commit().await.context("commit")?;
    Ok(Handover::Delivered)
}

/// The expiry for one order (the job calls it per claimed id): full refund with
/// stock back at the order's branch, no strike, in one transaction; then the
/// provider call. `Ok(false)`, with nothing changed, when the order is no longer
/// due: collected, held, its payment flagged, or refunded for another reason.
pub async fn expire_one(state: &AppState, order_id: Uuid) -> Result<bool, AppError> {
    let mut tx = state.db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    // begin_refund locks the variants and the branch's stock rows, then the order,
    // and refunds only a `paid` order: a collected one is a conflict, i.e. not due.
    let refund_id = match begin_refund(&mut tx, order_id, true, REFUND_REASON).await {
        Ok(id) => id,
        Err(AppError::Conflict("not_refundable" | "already_released")) => return Ok(false),
        Err(e) => return Err(e),
    };
    // Re-checked under the order lock begin_refund now holds, and the refund must be
    // the one this transaction made (an order a buyer cancel moved to
    // refund_pending returns that refund instead).
    let (due, store_id): (bool, Uuid) = sqlx::query_as(&format!(
        "SELECT {EXPIRY_SCOPE} AND r.reason = $2 AND r.created_at = now(), o.store_id \
         FROM orders o JOIN refunds r ON r.order_id = o.id \
              JOIN checkouts c ON c.id = o.checkout_id WHERE o.id = $1"
    ))
    .bind(order_id)
    .bind(REFUND_REASON)
    .fetch_one(&mut *tx)
    .await
    .context("recheck collection expiry")?;
    if !due {
        // Rolls back anything begin_refund changed.
        return Ok(false);
    }
    sqlx::query("UPDATE orders SET collect_by = NULL WHERE id = $1")
        .bind(order_id)
        .execute(&mut *tx)
        .await
        .context("clear collect deadline")?;
    audit::record(
        &mut tx,
        None,
        COLLECTION_EXPIRED,
        "order",
        Some(order_id),
        json!({ "refund_id": refund_id, "store_id": store_id }),
    )
    .await?;
    notify::tell_buyer(&mut tx, Kind::CollectionExpired, order_id).await?;
    notify::tell_team(&mut tx, Kind::CollectionExpired, order_id, Audience::Order).await?;
    tx.commit().await.context("commit collection expiry")?;
    // The refund is booked; a failed send stays `requested` for the sweeper.
    if let Err(e) = send_refund(state, refund_id).await {
        tracing::warn!(error = %e, "collection refund not sent; the sweeper retries");
    }
    Ok(true)
}

/// SQL over `orders o`: the order is due its day-`day` reminder now; `day` is the
/// SQL expression of the day (a cast parameter).
fn reminder_due(day: &str) -> String {
    format!(
        "o.delivery_method = 'collect' AND o.status = 'paid' AND o.held_at IS NULL \
         AND o.ready_at IS NOT NULL AND o.collect_by IS NOT NULL AND now() < o.collect_by \
         AND now() >= o.ready_at + make_interval(days => {day}) \
         AND ({day} = {last} OR now() < o.ready_at + make_interval(days => {last})) \
         AND NOT EXISTS (SELECT 1 FROM notifications n WHERE n.user_id = o.buyer_id \
             AND n.dedupe_key = 'collect_reminder:' || o.id || ':' || ({day})::text)",
        last = REMINDER_DAYS[REMINDER_DAYS.len() - 1],
    )
}

/// Job `collect_reminders`: raises each due day-1 and day-4 reminder once.
pub async fn send_reminders(
    state: &AppState,
    cancel: &CancellationToken,
) -> Result<Stats, AppError> {
    let mut stats = Stats::default();
    for day in REMINDER_DAYS {
        let due: Vec<Uuid> = sqlx::query_scalar(&format!(
            "SELECT o.id FROM orders o WHERE {} ORDER BY o.ready_at, o.id LIMIT $1",
            reminder_due("$2::int")
        ))
        .bind(BATCH)
        .bind(day)
        .fetch_all(&state.db)
        .await
        .context("find orders due a collect reminder")?;
        for order_id in due {
            if cancel.is_cancelled() {
                return Ok(stats);
            }
            match remind(state, order_id, day).await {
                Ok(true) => stats.done += 1,
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(job = COLLECT_REMINDERS, error = %e, "collect reminder not raised");
                    stats.failed += 1;
                }
            }
        }
    }
    Ok(stats)
}

/// One reminder, re-checked under the order lock (a collection or a refund may
/// have won). `false` when the order is locked by someone else or no longer due.
async fn remind(state: &AppState, order_id: Uuid, day: i32) -> Result<bool, AppError> {
    let mut tx = state.db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    let buyer: Option<Uuid> = sqlx::query_scalar(&format!(
        "SELECT o.buyer_id FROM orders o WHERE o.id = $1 AND {} FOR UPDATE OF o SKIP LOCKED",
        reminder_due("$2::int")
    ))
    .bind(order_id)
    .bind(day)
    .fetch_optional(&mut *tx)
    .await
    .context("lock order for collect reminder")?;
    let Some(buyer) = buyer else {
        return Ok(false);
    };
    let mut notice = Notice::order(buyer, Kind::CollectReminder, order_id);
    notice.dedupe_key = format!("{}:{order_id}:{day}", Kind::CollectReminder.as_str());
    enqueue(&mut tx, notice).await?;
    tx.commit().await.context("commit")?;
    Ok(true)
}
