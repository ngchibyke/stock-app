//! Partial refunds: a dispute decided partly for the buyer. Part of the items
//! total goes back to the buyer, the rest is released to the seller, in one
//! transaction under the order lock.
//!
//! Amounts (global constraints, "Partial refund amounts"):
//! - `retained = items_total_kobo − refund_kobo`, with `refund_kobo` in
//!   `1..items_total_kobo` (else 422 `refund_out_of_range`). The delivery fee
//!   is never refunded here: on a courier order the platform has already paid
//!   the courier; only a full refund returns it;
//! - `cut = money::cut_kobo(retained, platform_cut_bps)`, on the order's
//!   snapshotted bps;
//! - the release then runs the existing formula ([`release::release_amounts`])
//!   with `items = retained` and that `cut`. Delivery and the collection fee
//!   shares are the order's own: the seller still pays the collection fee, so
//!   there is no `refund_fee` posting.
//!
//! Ledger: `("refund", refund_id)` posts `escrow −refund_kobo`, `refunds
//! +refund_kobo`; then `("release", order_id)` posts on `retained + delivery`,
//! which empties the order's escrow. The order ends `completed` with its hold
//! cleared, its `platform_cut_kobo` is the cut on `retained`, and its `refunds` row has `kind = 'partial'` (reason
//! `dispute_partial`), so [`super::refund::send_refund`] and the owner's mark-refunded
//! leave the order `completed`.
//!
//! No provider call runs here: the caller commits, then sends the refund with
//! [`super::refund::send_refund`] and the payout with [`release::send_payout`].
//!
//! Lock order: the order is locked first (`orders`), then `release_in` locks
//! it again in the same transaction; the payout row is inserted after.

use anyhow::Context;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{
    audit,
    checkout::service::busy_err,
    error::AppError,
    ledger::{self, Account, Entry},
    money, payments,
    settings::Settings,
};

use super::release::{self, ReleaseReason, ReleasedIn};

const REFERENCE_PREFIX: &str = "RF";
const LOCK_TIMEOUT: &str = "SET LOCAL lock_timeout = '5s'";
/// `refunds.reason` of a partial refund.
pub const PARTIAL_REASON: &str = "dispute_partial";

fn db_err(ctx: &'static str) -> impl FnOnce(sqlx::Error) -> AppError {
    busy_err("order_busy", ctx)
}

/// What [`partial_refund_in`] booked inside the caller's transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialIn {
    /// The `requested` partial refund; the caller sends it after commit.
    pub refund_id: Uuid,
    /// The release of the retained part; the caller sends its payout after
    /// commit when it is `scheduled` and due.
    pub released: ReleasedIn,
}

#[derive(sqlx::FromRow)]
struct PartialRow {
    status: String,
    items_total_kobo: i64,
    platform_cut_bps: i32,
    platform_cut_kobo: i64,
    checkout_id: Uuid,
    provider: String,
    held: bool,
    has_payout: bool,
    has_refund: bool,
}

/// Refunds `refund_kobo` of a held shipped order's items to the buyer and
/// releases the rest to the seller (see the module docs), **inside the
/// caller's transaction**. Nothing is sent: after the commit the caller sends
/// the refund ([`super::refund::send_refund`]) and the payout
/// ([`release::send_payout`]), never inside the transaction.
///
/// Errors: unknown order 404; a `completed` order or one with a payout 409
/// `already_released`; an order not `picked_up`/`out_for_delivery`/`delivered`,
/// or with a refund already, 409 `invalid_order_state`; an unheld order 409
/// `order_not_held`; `refund_kobo` outside `1..items_total_kobo` 422
/// `refund_out_of_range`. Nothing is changed on an error (the caller's
/// transaction rolls back).
pub async fn partial_refund_in(
    conn: &mut PgConnection,
    settings: &Settings,
    order_id: Uuid,
    refund_kobo: i64,
) -> Result<PartialIn, AppError> {
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *conn)
        .await
        .map_err(db_err("set lock_timeout"))?;
    let row: PartialRow = sqlx::query_as(
        "SELECT o.status::text AS status, o.items_total_kobo, o.platform_cut_bps, \
                o.platform_cut_kobo, o.checkout_id, c.provider, o.held_at IS NOT NULL AS held, \
                EXISTS (SELECT 1 FROM payouts p WHERE p.order_id = o.id) AS has_payout, \
                EXISTS (SELECT 1 FROM refunds r WHERE r.order_id = o.id) AS has_refund \
         FROM orders o JOIN checkouts c ON c.id = o.checkout_id \
         WHERE o.id = $1 FOR UPDATE OF o",
    )
    .bind(order_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db_err("lock order"))?
    .ok_or(AppError::NotFound)?;
    let state_err = |status: &str| {
        AppError::ConflictDetail("invalid_order_state", format!("order is {status}"))
    };
    match row.status.as_str() {
        "picked_up" | "out_for_delivery" | "delivered" => {}
        "completed" => return Err(AppError::Conflict("already_released")),
        other => return Err(state_err(other)),
    }
    if row.has_payout {
        return Err(AppError::Conflict("already_released"));
    }
    if row.has_refund {
        return Err(state_err(&row.status));
    }
    if !row.held {
        return Err(AppError::Conflict("order_not_held"));
    }
    let items = row.items_total_kobo;
    if !(1..items).contains(&refund_kobo) {
        return Err(AppError::Unprocessable(
            "refund_out_of_range",
            format!("refund_kobo must be at least 1 and below {items}"),
        ));
    }
    let retained = items - refund_kobo;
    let cut = money::cut_kobo(retained, row.platform_cut_bps);
    // The order's cut becomes the one actually taken (the release books it,
    // and a released order's snapshot is never recomputed), so the seller's
    // view shows it beside the payout. The original is kept in the audit.
    sqlx::query("UPDATE orders SET platform_cut_kobo = $2 WHERE id = $1")
        .bind(order_id)
        .bind(cut)
        .execute(&mut *conn)
        .await
        .context("record partial cut")?;

    let reference = payments::new_reference(REFERENCE_PREFIX);
    let refund_id: Uuid = sqlx::query_scalar(
        "INSERT INTO refunds (order_id, reference, provider, amount_kobo, reason, kind) \
         VALUES ($1, $2, $3, $4, $5, 'partial') RETURNING id",
    )
    .bind(order_id)
    .bind(&reference)
    .bind(&row.provider)
    .bind(refund_kobo)
    .bind(PARTIAL_REASON)
    .fetch_one(&mut *conn)
    .await
    .context("insert partial refund")?;
    let entry = |account, amount_kobo| Entry {
        order_id: Some(order_id),
        checkout_id: Some(row.checkout_id),
        account,
        amount_kobo,
        kind: "refund",
        reference: Some(reference.clone()),
    };
    ledger::post(
        &mut *conn,
        ("refund", &refund_id.to_string()),
        &[
            entry(Account::Escrow, -refund_kobo),
            entry(Account::Refunds, refund_kobo),
        ],
    )
    .await?;

    let released = release::release_in(
        &mut *conn,
        settings,
        order_id,
        ReleaseReason::OwnerDecision,
        Some((retained, cut)),
    )
    .await?;
    if let ReleasedIn::Unchanged(status) = &released {
        // Checked above under the same lock; cannot happen.
        return Err(state_err(status));
    }
    // The decision ends the complaint: the hold goes, so the payout can go.
    sqlx::query("UPDATE orders SET held_at = NULL, hold_reason = NULL WHERE id = $1")
        .bind(order_id)
        .execute(&mut *conn)
        .await
        .context("clear hold")?;
    audit::record(
        &mut *conn,
        None,
        "order.partial_refund",
        "order",
        Some(order_id),
        json!({
            "refund_id": refund_id,
            "refund_kobo": refund_kobo,
            "retained_kobo": retained,
            "cut_kobo": cut,
            "original_cut_kobo": row.platform_cut_kobo,
        }),
    )
    .await?;
    Ok(PartialIn {
        refund_id,
        released,
    })
}
