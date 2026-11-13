//! The staff dispute screen: one read-only view of everything the market needs
//! to decide a dispute, and the queue that leads to it.
//!
//! **Who sees what.** The Owner reads any dispute, including the original text
//! of the chat's flagged messages. Support reads open disputes only (403
//! `dispute_closed` otherwise) and never an original: `flagged` is left out of
//! its response altogether, and so is the order's hold reason.
//!
//! **Personal data.** The screen shows the delivery *area council* and nothing
//! else of the buyer: no name, phone or address. The handover section says
//! whether a code was entered wrongly or used, never what the code is. The
//! chat is delivered messages only. Every read writes `admin.dispute_viewed`
//! (`{dispute_id, originals}`, never text) before the response is returned.

use anyhow::Context;
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::service::{self, DisputeStatus, DisputeView};
use crate::{
    AppState,
    admin::{
        Staff,
        views::{self, AdminOrderView},
    },
    audit,
    chat::service::{FlaggedOriginal, StaffMessage, flagged_originals, staff_transcript},
    delivery::{
        area::AreaCouncil,
        tracking::{Step, staff_timeline},
    },
    error::AppError,
};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct QueueRow {
    pub id: Uuid,
    pub order_id: Uuid,
    pub status: String,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub opened_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub seller_respond_by: OffsetDateTime,
    pub items_total_kobo: i64,
}

/// Disputes, oldest first, optionally of one status.
pub async fn queue(
    db: &PgPool,
    status: Option<DisputeStatus>,
    limit: i64,
) -> Result<Vec<QueueRow>, AppError> {
    Ok(sqlx::query_as(
        "SELECT d.id, d.order_id, d.status, d.reason, d.opened_at, d.seller_respond_by, \
                o.items_total_kobo \
         FROM disputes d JOIN orders o ON o.id = d.order_id \
         WHERE ($1::text IS NULL OR d.status = $1) \
         ORDER BY d.opened_at, d.id LIMIT $2",
    )
    .bind(status.map(DisputeStatus::as_str))
    .bind(limit)
    .fetch_all(db)
    .await
    .context("list disputes")?)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PaymentView {
    pub checkout_status: String,
    pub provider: String,
    pub provider_flagged: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    pub paid_at: Option<OffsetDateTime>,
    pub total_kobo: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct LedgerRow {
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub account: String,
    pub amount_kobo: i64,
    pub kind: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DeliveryEventRow {
    pub source: String,
    pub status: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub occurred_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: OffsetDateTime,
}

/// The handover code's story, never the code.
#[derive(Debug, Serialize)]
pub struct HandoverView {
    /// Only a self-delivery order has a handover code.
    pub applies: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    pub issued_at: Option<OffsetDateTime>,
    pub wrong_entries: i64,
    #[serde(with = "time::serde::rfc3339::option")]
    pub locked_until: Option<OffsetDateTime>,
    pub delivered_with_code: bool,
}

#[derive(Debug, Serialize)]
pub struct StaffDisputeView {
    pub dispute: DisputeView,
    pub order: AdminOrderView,
    pub payment: PaymentView,
    pub ledger: Vec<LedgerRow>,
    pub timeline: Vec<Step>,
    pub delivery_events: Vec<DeliveryEventRow>,
    pub handover: HandoverView,
    pub delivery_area: Option<AreaCouncil>,
    pub chat: Vec<StaffMessage>,
    /// Owner only; absent for Support.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flagged: Option<Vec<FlaggedOriginal>>,
}

async fn payment(db: &PgPool, order_id: Uuid) -> Result<PaymentView, AppError> {
    Ok(sqlx::query_as(
        "SELECT c.status::text AS checkout_status, c.provider, c.provider_flagged, c.paid_at, \
                c.total_kobo \
         FROM orders o JOIN checkouts c ON c.id = o.checkout_id WHERE o.id = $1",
    )
    .bind(order_id)
    .fetch_one(db)
    .await
    .context("load dispute payment")?)
}

type OrderFacts = (
    bool,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    Option<AreaCouncil>,
);

/// The handover story and the area council. A wrong entry is an
/// `order.handover_attempt` that failed (a locked attempt counts too).
async fn handover(
    db: &PgPool,
    order_id: Uuid,
) -> Result<(HandoverView, Option<AreaCouncil>), AppError> {
    let (is_self, issued_at, locked_until, area): OrderFacts = sqlx::query_as(
        "SELECT COALESCE(delivery_method::text = 'self', false), out_for_delivery_at, \
                handover_locked_until, delivery_area FROM orders WHERE id = $1",
    )
    .bind(order_id)
    .fetch_one(db)
    .await
    .context("load handover facts")?;
    let (wrong, with_code): (i64, bool) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE action = 'order.handover_attempt' \
                                   AND detail->>'ok' = 'false'), \
                COALESCE(bool_or(action = 'order.delivered' \
                                   AND detail->>'via' = 'handover_code'), false) \
         FROM audit_log WHERE subject_type = 'order' AND subject_id = $1 \
           AND action IN ('order.handover_attempt', 'order.delivered')",
    )
    .bind(order_id)
    .fetch_one(db)
    .await
    .context("count handover attempts")?;
    Ok((
        HandoverView {
            applies: is_self,
            issued_at: issued_at.filter(|_| is_self),
            wrong_entries: wrong,
            locked_until: locked_until.filter(|_| is_self),
            delivered_with_code: with_code,
        },
        area,
    ))
}

/// The screen for `dispute_id`. Support: open disputes only, no originals.
/// The read is audited before this returns.
pub async fn screen(
    state: &AppState,
    staff: &Staff,
    dispute_id: Uuid,
) -> Result<StaffDisputeView, AppError> {
    let owner = staff.is_owner();
    let dispute = {
        let mut conn = state.db.acquire().await.context("acquire")?;
        service::load(&mut conn, state, dispute_id).await?.0
    };
    let open = DisputeStatus::parse(&dispute.status).is_some_and(DisputeStatus::is_open);
    if !owner && !open {
        return Err(AppError::Forbidden("dispute_closed"));
    }
    let order_id = dispute.order_id;
    let db = &state.db;

    let mut order = views::order(db, order_id).await?;
    if !owner {
        // The owner's free text.
        order.hold_reason = None;
    }
    let ledger = sqlx::query_as(
        "SELECT at, account::text AS account, amount_kobo, kind FROM ledger_entries \
         WHERE order_id = $1 ORDER BY id",
    )
    .bind(order_id)
    .fetch_all(db)
    .await
    .context("load dispute ledger")?;
    let delivery_events = sqlx::query_as(
        "SELECT source, status, occurred_at, received_at FROM delivery_events \
         WHERE order_id = $1 ORDER BY received_at, id",
    )
    .bind(order_id)
    .fetch_all(db)
    .await
    .context("load dispute delivery events")?;
    let (handover, delivery_area) = handover(db, order_id).await?;
    let view = StaffDisputeView {
        payment: payment(db, order_id).await?,
        timeline: staff_timeline(db, order_id).await?,
        chat: staff_transcript(db, &state.cipher, order_id).await?,
        flagged: if owner {
            Some(flagged_originals(db, &state.cipher, order_id).await?)
        } else {
            None
        },
        dispute,
        order,
        ledger,
        delivery_events,
        handover,
        delivery_area,
    };

    // Fail closed: nothing is returned that could not be audited.
    let mut tx = db.begin().await.context("begin")?;
    audit::record(
        &mut tx,
        Some(staff.0.id),
        "admin.dispute_viewed",
        "dispute",
        Some(dispute_id),
        json!({ "dispute_id": dispute_id, "originals": owner }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(view)
}
