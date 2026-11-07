//! The owner's decision on a dispute: a full refund, a release, or a partial
//! refund. One transaction, in the global lock order: the order is locked,
//! then the dispute is marked decided (a state-conditional update, so a second
//! decision racing this one finds it closed), then the escrow engine's
//! in-transaction step books the money. Nothing is sent inside the
//! transaction: after the commit the refund and/or payout are sent, and a
//! failed send is only logged, because the sweepers retry.
//!
//! The dispute is marked decided before the engine runs, and the engine's
//! in-transaction steps are called directly, so the open-dispute guards on
//! the owner's plain release, refund and unhold never see this path.

use anyhow::Context;
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use super::service::{DisputeView, begin_locked, load};
use crate::{
    AppState, audit,
    checkout::service::busy_err,
    error::AppError,
    escrow::{
        partial::{PartialIn, partial_refund_in},
        refund::{owner_begin_in, send_refund},
        release::{ReleaseReason, ReleasedIn, release_in, send_payout},
    },
    settings,
};

/// What the owner decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Items plus delivery back to the buyer; stock not restored.
    FullRefund,
    /// `refund_kobo` of the items back to the buyer, the rest released.
    PartialRefund(i64),
    /// Everything released to the seller.
    Release,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::FullRefund => "full_refund",
            Decision::PartialRefund(_) => "partial_refund",
            Decision::Release => "release",
        }
    }

    pub fn refund_kobo(self) -> Option<i64> {
        match self {
            Decision::PartialRefund(kobo) => Some(kobo),
            _ => None,
        }
    }
}

/// After the commit: a payout that is `scheduled` and due.
fn due_payout(released: &ReleasedIn) -> Option<Uuid> {
    match released {
        ReleasedIn::Done {
            payout_id,
            status,
            send_after,
        } if *status == "scheduled" && *send_after <= OffsetDateTime::now_utc() => Some(*payout_id),
        _ => None,
    }
}

/// Settles the dispute (see the module docs). `note` is validated by the
/// caller and kept in the `admin.dispute_decided` audit row.
///
/// Errors: unknown dispute 404; `awaiting_seller` before the seller's
/// deadline 409 `awaiting_seller`; a decided or withdrawn dispute 409
/// `dispute_closed`, naming the state; a partial `refund_kobo` outside
/// `1..items_total_kobo` 422 `refund_out_of_range`; an order the engine will
/// not move 409 `invalid_order_state` (or the engine's own code). Nothing is
/// changed on an error.
pub async fn decide(
    state: &AppState,
    actor: Uuid,
    dispute_id: Uuid,
    decision: Decision,
    note: &str,
) -> Result<DisputeView, AppError> {
    // Outside the transaction, as `release` does.
    let order_id: Uuid = sqlx::query_scalar("SELECT order_id FROM disputes WHERE id = $1")
        .bind(dispute_id)
        .fetch_optional(&state.db)
        .await
        .context("find dispute")?
        .ok_or(AppError::NotFound)?;
    let settings = settings::load(&state.db).await?;

    let mut tx = begin_locked(state).await?;
    // Orders before disputes (global lock order).
    let items_total: i64 =
        sqlx::query_scalar("SELECT items_total_kobo FROM orders WHERE id = $1 FOR UPDATE")
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(busy_err("order_busy", "lock order"))?;
    // Checked before the dispute row is written: its CHECK wants a positive
    // amount, and the engine checks the same range again under the same lock.
    if let Decision::PartialRefund(kobo) = decision
        && !(1..items_total).contains(&kobo)
    {
        return Err(AppError::Unprocessable(
            "refund_out_of_range",
            format!("refund_kobo must be at least 1 and below {items_total}"),
        ));
    }

    // A dispute still awaiting the seller may be decided only once the
    // seller's deadline has passed; the deadline job may not have run yet,
    // so a missed deadline is recorded here too.
    let marked: Option<Uuid> = sqlx::query_scalar(
        "UPDATE disputes SET status = 'decided', decision = $2, refund_kobo = $3, \
                decided_by = $4, decided_at = now(), \
                seller_missed_deadline = (seller_missed_deadline OR status = 'awaiting_seller') \
         WHERE id = $1 AND (status IN ('awaiting_decision', 'awaiting_return') \
                OR (status = 'awaiting_seller' AND seller_respond_by <= now())) \
         RETURNING id",
    )
    .bind(dispute_id)
    .bind(decision.as_str())
    .bind(decision.refund_kobo())
    .bind(actor)
    .fetch_optional(&mut *tx)
    .await
    .map_err(busy_err("dispute_busy", "decide dispute"))?;
    if marked.is_none() {
        let status: String = sqlx::query_scalar("SELECT status FROM disputes WHERE id = $1")
            .bind(dispute_id)
            .fetch_one(&mut *tx)
            .await
            .context("load dispute status")?;
        let code = if status == "awaiting_seller" {
            "awaiting_seller"
        } else {
            "dispute_closed"
        };
        return Err(AppError::ConflictDetail(
            code,
            format!("dispute is {status}"),
        ));
    }

    let unchanged = |status: String| {
        AppError::ConflictDetail("invalid_order_state", format!("order is {status}"))
    };
    let (refund_id, payout_id) = match decision {
        Decision::Release => {
            let released = release_in(
                &mut tx,
                &settings,
                order_id,
                ReleaseReason::OwnerDecision,
                None,
            )
            .await?;
            if let ReleasedIn::Unchanged(status) = released {
                return Err(unchanged(status));
            }
            // The decision ends the complaint: the hold goes, so the payout can go.
            sqlx::query("UPDATE orders SET held_at = NULL, hold_reason = NULL WHERE id = $1")
                .bind(order_id)
                .execute(&mut *tx)
                .await
                .context("clear hold")?;
            (None, due_payout(&released))
        }
        Decision::FullRefund => (Some(owner_begin_in(&mut tx, order_id).await?), None),
        Decision::PartialRefund(kobo) => {
            let PartialIn {
                refund_id,
                released,
            } = partial_refund_in(&mut tx, &settings, order_id, kobo).await?;
            if let ReleasedIn::Unchanged(status) = released {
                return Err(unchanged(status));
            }
            (Some(refund_id), due_payout(&released))
        }
    };
    audit::record(
        &mut tx,
        Some(actor),
        "admin.dispute_decided",
        "order",
        Some(order_id),
        json!({
            "dispute_id": dispute_id,
            "decision": decision.as_str(),
            "refund_kobo": decision.refund_kobo(),
            "note": note,
        }),
    )
    .await?;
    tx.commit().await.context("commit decision")?;

    // After the commit; the sweepers retry anything that fails here.
    if let Some(refund_id) = refund_id
        && let Err(e) = send_refund(state, refund_id).await
    {
        tracing::warn!(error = %e, "refund after dispute decision not sent; the job retries");
    }
    if let Some(payout_id) = payout_id
        && let Err(e) = send_payout(state, payout_id).await
    {
        tracing::warn!(error = %e, "payout after dispute decision not sent; the job retries");
    }

    let mut conn = state.db.acquire().await.context("acquire")?;
    let (view, _, _) = load(&mut conn, state, dispute_id).await?;
    Ok(view)
}
