//! Opening, viewing and withdrawing a dispute.
//!
//! Opening locks the order (orders before disputes, the global lock order) and
//! holds it, so the auto-release job, the buyer's "Received" and payout sends
//! all stop. Statements are encrypted at rest and checked offline only
//! (`moderation::filter::scan`): no strike, no rewrite and no third-party call,
//! because complaints run hot and evidence stays with us. The seller only ever
//! sees "the buyer": no name, phone or address is in [`DisputeView`], and the
//! owner's private note never is.

use anyhow::Context;
use serde::Serialize;
use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    admin::LOCK_TIMEOUT,
    audit,
    checkout::service::busy_err,
    error::AppError,
    limits::{MAX_DISPUTE_CHARS, MAX_DISPUTE_PHOTOS},
    moderation::filter::{Category, Scan, Severity, scan},
};

/// Calendar hours the seller has to respond.
pub const SELLER_RESPONSE_HOURS: i64 = 48;

const CONTACT_COPY: &str =
    "Keep contact details and payments inside the app. Remove them and try again.";
const ABUSE_COPY: &str = "This breaks the market's rules. Describe the problem without insults.";
pub const WITHDRAW_NOTE: &str = "You can't report this order again after withdrawing.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    NotReceived,
    Damaged,
    WrongItem,
    NotAsDescribed,
    Incomplete,
    Other,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::NotReceived => "not_received",
            Reason::Damaged => "damaged",
            Reason::WrongItem => "wrong_item",
            Reason::NotAsDescribed => "not_as_described",
            Reason::Incomplete => "incomplete",
            Reason::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "not_received" => Reason::NotReceived,
            "damaged" => Reason::Damaged,
            "wrong_item" => Reason::WrongItem,
            "not_as_described" => Reason::NotAsDescribed,
            "incomplete" => Reason::Incomplete,
            "other" => Reason::Other,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisputeStatus {
    AwaitingSeller,
    AwaitingReturn,
    AwaitingDecision,
    Decided,
    Withdrawn,
}

impl DisputeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DisputeStatus::AwaitingSeller => "awaiting_seller",
            DisputeStatus::AwaitingReturn => "awaiting_return",
            DisputeStatus::AwaitingDecision => "awaiting_decision",
            DisputeStatus::Decided => "decided",
            DisputeStatus::Withdrawn => "withdrawn",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "awaiting_seller" => DisputeStatus::AwaitingSeller,
            "awaiting_return" => DisputeStatus::AwaitingReturn,
            "awaiting_decision" => DisputeStatus::AwaitingDecision,
            "decided" => DisputeStatus::Decided,
            "withdrawn" => DisputeStatus::Withdrawn,
            _ => return None,
        })
    }

    /// Open: the order's money is still frozen by this dispute.
    pub fn is_open(self) -> bool {
        matches!(
            self,
            DisputeStatus::AwaitingSeller
                | DisputeStatus::AwaitingReturn
                | DisputeStatus::AwaitingDecision
        )
    }
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub body: String,
    pub photo_keys: Vec<String>,
}

/// Limits and the offline scan. A serious hit of any category is refused with
/// no strike and no Claude call.
pub fn check_statement(s: &Statement) -> Result<(), AppError> {
    let len = s.body.trim().chars().count();
    if len == 0 || len > MAX_DISPUTE_CHARS {
        return Err(AppError::Validation(format!(
            "body must be 1 to {MAX_DISPUTE_CHARS} characters"
        )));
    }
    if s.photo_keys.len() > MAX_DISPUTE_PHOTOS {
        return Err(AppError::Validation(format!(
            "photo_keys must contain at most {MAX_DISPUTE_PHOTOS} entries"
        )));
    }
    if s.photo_keys.iter().any(|k| k.trim().is_empty()) {
        return Err(AppError::Validation(
            "photo_keys must not contain empty entries".into(),
        ));
    }
    if let Scan::Hit {
        severity: Severity::Serious,
        category,
    } = scan(&s.body)
    {
        let copy = if category == Category::Abuse {
            ABUSE_COPY
        } else {
            CONTACT_COPY
        };
        return Err(AppError::Unprocessable("statement_refused", copy.into()));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct StatementView {
    pub body: String,
    pub photo_keys: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
pub struct ReturnView {
    pub paid_by: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub requested_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub received_at: Option<OffsetDateTime>,
}

#[derive(Debug, Serialize)]
pub struct DecisionView {
    pub decision: String,
    pub refund_kobo: Option<i64>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub decided_at: Option<OffsetDateTime>,
}

/// What either party sees. No name, phone, address or owner note.
#[derive(Debug, Serialize)]
pub struct DisputeView {
    pub id: Uuid,
    pub order_id: Uuid,
    pub status: String,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub opened_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub seller_respond_by: OffsetDateTime,
    pub seller_missed_deadline: bool,
    pub buyer_statement: StatementView,
    pub seller_statement: Option<StatementView>,
    pub return_request: Option<ReturnView>,
    pub decision: Option<DecisionView>,
}

#[derive(Debug, Serialize)]
pub struct WithdrawResponse {
    #[serde(flatten)]
    pub dispute: DisputeView,
    pub note: &'static str,
}

#[derive(sqlx::FromRow)]
struct DisputeRow {
    id: Uuid,
    order_id: Uuid,
    buyer_id: Uuid,
    owner_id: Uuid,
    status: String,
    reason: String,
    opened_at: OffsetDateTime,
    seller_respond_by: OffsetDateTime,
    seller_missed_deadline: bool,
    return_paid_by: Option<String>,
    return_requested_at: Option<OffsetDateTime>,
    return_received_at: Option<OffsetDateTime>,
    decision: Option<String>,
    refund_kobo: Option<i64>,
    decided_at: Option<OffsetDateTime>,
}

/// The dispute as the parties see it, plus the two people allowed to see it.
async fn load(
    conn: &mut PgConnection,
    state: &AppState,
    dispute_id: Uuid,
) -> Result<(DisputeView, Uuid, Uuid), AppError> {
    let d: DisputeRow = sqlx::query_as(
        "SELECT d.id, d.order_id, d.buyer_id, s.owner_id, d.status, d.reason, d.opened_at, \
                d.seller_respond_by, d.seller_missed_deadline, d.return_paid_by, \
                d.return_requested_at, d.return_received_at, d.decision, d.refund_kobo, \
                d.decided_at \
         FROM disputes d JOIN stores s ON s.id = d.store_id WHERE d.id = $1",
    )
    .bind(dispute_id)
    .fetch_optional(&mut *conn)
    .await
    .context("load dispute")?
    .ok_or(AppError::NotFound)?;
    let rows: Vec<(String, Vec<u8>, Vec<String>, OffsetDateTime)> = sqlx::query_as(
        "SELECT party, body_enc, photo_keys, created_at FROM dispute_statements \
         WHERE dispute_id = $1",
    )
    .bind(dispute_id)
    .fetch_all(&mut *conn)
    .await
    .context("load dispute statements")?;
    let mut buyer = None;
    let mut seller = None;
    for (party, enc, photo_keys, at) in rows {
        let statement = StatementView {
            body: state.cipher.decrypt(&enc)?,
            photo_keys,
            at,
        };
        match party.as_str() {
            "buyer" => buyer = Some(statement),
            _ => seller = Some(statement),
        }
    }
    let view = DisputeView {
        id: d.id,
        order_id: d.order_id,
        status: d.status,
        reason: d.reason,
        opened_at: d.opened_at,
        seller_respond_by: d.seller_respond_by,
        seller_missed_deadline: d.seller_missed_deadline,
        buyer_statement: buyer.context("dispute without a buyer statement")?,
        seller_statement: seller,
        return_request: d.return_paid_by.map(|paid_by| ReturnView {
            paid_by,
            requested_at: d.return_requested_at,
            received_at: d.return_received_at,
        }),
        decision: d.decision.map(|decision| DecisionView {
            decision,
            refund_kobo: d.refund_kobo,
            decided_at: d.decided_at,
        }),
    };
    Ok((view, d.buyer_id, d.owner_id))
}

async fn begin_locked(
    state: &AppState,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, AppError> {
    let mut tx = state.db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    Ok(tx)
}

#[derive(sqlx::FromRow)]
struct OrderLock {
    store_id: Uuid,
    status: String,
    held: bool,
    has_payout: bool,
}

/// The id of the order's open dispute, if any.
pub async fn open_for_order(
    conn: &mut PgConnection,
    order_id: Uuid,
) -> Result<Option<Uuid>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM disputes WHERE order_id = $1 \
         AND status IN ('awaiting_seller', 'awaiting_return', 'awaiting_decision')",
    )
    .bind(order_id)
    .fetch_optional(conn)
    .await
    .context("find open dispute")?)
}

/// The buyer reports a problem. Holds the order unless it is already held.
pub async fn open(
    state: &AppState,
    buyer: Uuid,
    order_id: Uuid,
    reason: Reason,
    s: Statement,
) -> Result<DisputeView, AppError> {
    check_statement(&s)?;
    let mut tx = begin_locked(state).await?;
    let order: OrderLock = sqlx::query_as(
        "SELECT o.store_id, o.status::text AS status, o.held_at IS NOT NULL AS held, \
                EXISTS (SELECT 1 FROM payouts p WHERE p.order_id = o.id) AS has_payout \
         FROM orders o WHERE o.id = $1 AND o.buyer_id = $2 FOR UPDATE OF o",
    )
    .bind(order_id)
    .bind(buyer)
    .fetch_optional(&mut *tx)
    .await
    .map_err(busy_err("order_busy", "lock order"))?
    .ok_or(AppError::NotFound)?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM disputes WHERE order_id = $1)")
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await
            .context("check dispute")?;
    if exists {
        return Err(AppError::Conflict("dispute_exists"));
    }
    match order.status.as_str() {
        "picked_up" | "out_for_delivery" | "delivered" if !order.has_payout => {}
        "paid" => return Err(AppError::Conflict("cancel_instead")),
        "completed" | "picked_up" | "out_for_delivery" | "delivered" => {
            return Err(AppError::Conflict("already_released"));
        }
        other => {
            return Err(AppError::ConflictDetail(
                "invalid_order_state",
                format!("order is {other}"),
            ));
        }
    }
    let dispute_id: Uuid = sqlx::query_scalar(
        "INSERT INTO disputes (order_id, buyer_id, store_id, reason, status, seller_respond_by) \
         VALUES ($1, $2, $3, $4, 'awaiting_seller', now() + make_interval(hours => $5)) \
         RETURNING id",
    )
    .bind(order_id)
    .bind(buyer)
    .bind(order.store_id)
    .bind(reason.as_str())
    .bind(SELLER_RESPONSE_HOURS as i32)
    .fetch_one(&mut *tx)
    .await
    .context("insert dispute")?;
    sqlx::query(
        "INSERT INTO dispute_statements (dispute_id, party, body_enc, photo_keys) \
         VALUES ($1, 'buyer', $2, $3)",
    )
    .bind(dispute_id)
    .bind(state.cipher.encrypt(s.body.trim()))
    .bind(&s.photo_keys)
    .execute(&mut *tx)
    .await
    .context("insert dispute statement")?;
    if !order.held {
        sqlx::query("UPDATE orders SET held_at = now(), hold_reason = 'dispute' WHERE id = $1")
            .bind(order_id)
            .execute(&mut *tx)
            .await
            .context("hold order")?;
    }
    audit::record(
        &mut tx,
        Some(buyer),
        "dispute.opened",
        "dispute",
        Some(dispute_id),
        json!({ "reason": reason.as_str(), "prior_status": order.status }),
    )
    .await?;
    let (view, _, _) = load(&mut tx, state, dispute_id).await?;
    tx.commit().await.context("commit")?;
    Ok(view)
}

/// The buyer withdraws an open report. Final: see [`WITHDRAW_NOTE`].
pub async fn withdraw(
    state: &AppState,
    buyer: Uuid,
    dispute_id: Uuid,
) -> Result<WithdrawResponse, AppError> {
    // Find the order first: orders are locked before disputes.
    let order_id: Uuid =
        sqlx::query_scalar("SELECT order_id FROM disputes WHERE id = $1 AND buyer_id = $2")
            .bind(dispute_id)
            .bind(buyer)
            .fetch_optional(&state.db)
            .await
            .context("find dispute")?
            .ok_or(AppError::NotFound)?;
    let mut tx = begin_locked(state).await?;
    let hold_reason: Option<String> =
        sqlx::query_scalar("SELECT hold_reason FROM orders WHERE id = $1 FOR UPDATE")
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(busy_err("order_busy", "lock order"))?;
    let status: String = sqlx::query_scalar("SELECT status FROM disputes WHERE id = $1 FOR UPDATE")
        .bind(dispute_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(busy_err("dispute_busy", "lock dispute"))?;
    if !DisputeStatus::parse(&status).is_some_and(DisputeStatus::is_open) {
        return Err(AppError::Conflict("dispute_closed"));
    }
    sqlx::query("UPDATE disputes SET status = 'withdrawn', withdrawn_at = now() WHERE id = $1")
        .bind(dispute_id)
        .execute(&mut *tx)
        .await
        .context("withdraw dispute")?;
    // An owner's hold placed first is the owner's to lift.
    let unheld = hold_reason.as_deref() == Some("dispute");
    if unheld {
        sqlx::query("UPDATE orders SET held_at = NULL, hold_reason = NULL WHERE id = $1")
            .bind(order_id)
            .execute(&mut *tx)
            .await
            .context("unhold order")?;
    }
    audit::record(
        &mut tx,
        Some(buyer),
        "dispute.withdrawn",
        "dispute",
        Some(dispute_id),
        json!({ "prior_status": status, "unheld": unheld }),
    )
    .await?;
    let (dispute, _, _) = load(&mut tx, state, dispute_id).await?;
    tx.commit().await.context("commit")?;
    Ok(WithdrawResponse {
        dispute,
        note: WITHDRAW_NOTE,
    })
}

/// The dispute for its buyer or the store's owner; anyone else gets 404.
pub async fn view(
    state: &AppState,
    viewer: Uuid,
    dispute_id: Uuid,
) -> Result<DisputeView, AppError> {
    let mut conn = state.db.acquire().await.context("acquire")?;
    let (view, buyer, owner) = load(&mut conn, state, dispute_id).await?;
    if viewer != buyer && viewer != owner {
        return Err(AppError::NotFound);
    }
    Ok(view)
}

/// Who pays for the return shipping (the side at fault). Nothing moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaidBy {
    Buyer,
    Seller,
}

impl PaidBy {
    pub fn as_str(self) -> &'static str {
        match self {
            PaidBy::Buyer => "buyer",
            PaidBy::Seller => "seller",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "buyer" => Some(PaidBy::Buyer),
            "seller" => Some(PaidBy::Seller),
            _ => None,
        }
    }
}

/// The dispute's order, for the order-before-dispute lock order. `owner` of
/// `Some` restricts to that store owner (404 otherwise).
async fn order_of(
    state: &AppState,
    dispute_id: Uuid,
    owner: Option<Uuid>,
) -> Result<Uuid, AppError> {
    sqlx::query_scalar(
        "SELECT d.order_id FROM disputes d JOIN stores s ON s.id = d.store_id \
         WHERE d.id = $1 AND ($2::uuid IS NULL OR s.owner_id = $2)",
    )
    .bind(dispute_id)
    .bind(owner)
    .fetch_optional(&state.db)
    .await
    .context("find dispute")?
    .ok_or(AppError::NotFound)
}

#[derive(sqlx::FromRow)]
struct Locked {
    status: String,
    late: bool,
    responded: bool,
    missed: bool,
}

/// Locks the order, then the dispute, in a new transaction.
async fn lock_both(
    state: &AppState,
    order_id: Uuid,
    dispute_id: Uuid,
) -> Result<(sqlx::Transaction<'static, sqlx::Postgres>, Locked), AppError> {
    let mut tx = begin_locked(state).await?;
    sqlx::query("SELECT 1 FROM orders WHERE id = $1 FOR UPDATE")
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(busy_err("order_busy", "lock order"))?;
    let locked: Locked = sqlx::query_as(
        "SELECT status, now() > seller_respond_by AS late, \
                seller_responded_at IS NOT NULL AS responded, seller_missed_deadline AS missed \
         FROM disputes WHERE id = $1 FOR UPDATE",
    )
    .bind(dispute_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(busy_err("dispute_busy", "lock dispute"))?;
    Ok((tx, locked))
}

/// The seller answers once, before `seller_respond_by`.
pub async fn respond(
    state: &AppState,
    seller: Uuid,
    dispute_id: Uuid,
    s: Statement,
) -> Result<DisputeView, AppError> {
    let order_id = order_of(state, dispute_id, Some(seller)).await?;
    check_statement(&s)?;
    let (mut tx, d) = lock_both(state, order_id, dispute_id).await?;
    if d.responded {
        return Err(AppError::Conflict("already_responded"));
    }
    match DisputeStatus::parse(&d.status) {
        Some(DisputeStatus::AwaitingSeller) if !d.late => {}
        Some(DisputeStatus::AwaitingSeller) => {
            return Err(AppError::Conflict("response_closed"));
        }
        _ if d.missed => return Err(AppError::Conflict("response_closed")),
        Some(st) if st.is_open() => return Err(AppError::Conflict("response_closed")),
        _ => return Err(AppError::Conflict("dispute_closed")),
    }
    sqlx::query(
        "INSERT INTO dispute_statements (dispute_id, party, body_enc, photo_keys) \
         VALUES ($1, 'seller', $2, $3)",
    )
    .bind(dispute_id)
    .bind(state.cipher.encrypt(s.body.trim()))
    .bind(&s.photo_keys)
    .execute(&mut *tx)
    .await
    .context("insert seller statement")?;
    sqlx::query(
        "UPDATE disputes SET status = 'awaiting_decision', seller_responded_at = now() \
         WHERE id = $1",
    )
    .bind(dispute_id)
    .execute(&mut *tx)
    .await
    .context("record response")?;
    audit::record(
        &mut tx,
        Some(seller),
        "dispute.seller_responded",
        "dispute",
        Some(dispute_id),
        json!({ "photos": s.photo_keys.len() }),
    )
    .await?;
    let (view, _, _) = load(&mut tx, state, dispute_id).await?;
    tx.commit().await.context("commit")?;
    Ok(view)
}

/// Staff ask for the item back before deciding. No money moves.
pub async fn request_return(
    state: &AppState,
    actor: Uuid,
    dispute_id: Uuid,
    paid_by: PaidBy,
    note: &str,
) -> Result<(), AppError> {
    let order_id = order_of(state, dispute_id, None).await?;
    let (mut tx, d) = lock_both(state, order_id, dispute_id).await?;
    match DisputeStatus::parse(&d.status) {
        Some(DisputeStatus::AwaitingDecision) => {}
        Some(DisputeStatus::AwaitingSeller) => return Err(AppError::Conflict("awaiting_seller")),
        Some(DisputeStatus::AwaitingReturn) => {
            return Err(AppError::Conflict("return_already_requested"));
        }
        _ => return Err(AppError::Conflict("dispute_closed")),
    }
    sqlx::query(
        "UPDATE disputes SET status = 'awaiting_return', return_paid_by = $2, \
                return_requested_at = now(), return_received_at = NULL WHERE id = $1",
    )
    .bind(dispute_id)
    .bind(paid_by.as_str())
    .execute(&mut *tx)
    .await
    .context("request return")?;
    audit::record(
        &mut tx,
        Some(actor),
        "admin.dispute_return_requested",
        "dispute",
        Some(dispute_id),
        json!({ "paid_by": paid_by.as_str(), "note": note }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(())
}

/// The seller confirms the item is back; the dispute returns to the queue.
pub async fn return_received(
    state: &AppState,
    seller: Uuid,
    dispute_id: Uuid,
) -> Result<DisputeView, AppError> {
    let order_id = order_of(state, dispute_id, Some(seller)).await?;
    let (mut tx, d) = lock_both(state, order_id, dispute_id).await?;
    match DisputeStatus::parse(&d.status) {
        Some(DisputeStatus::AwaitingReturn) => {}
        Some(st) if st.is_open() => return Err(AppError::Conflict("not_awaiting_return")),
        _ => return Err(AppError::Conflict("dispute_closed")),
    }
    sqlx::query(
        "UPDATE disputes SET status = 'awaiting_decision', return_received_at = now() \
         WHERE id = $1",
    )
    .bind(dispute_id)
    .execute(&mut *tx)
    .await
    .context("record return")?;
    audit::record(
        &mut tx,
        Some(seller),
        "dispute.return_received",
        "dispute",
        Some(dispute_id),
        json!({}),
    )
    .await?;
    let (view, _, _) = load(&mut tx, state, dispute_id).await?;
    tx.commit().await.context("commit")?;
    Ok(view)
}
