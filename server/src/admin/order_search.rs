//! Dashboard order search and order detail with the ledger (plan 6, task 4).
//!
//! Owner only. Neither the list nor the detail carries buyer data: no buyer id,
//! name, phone or address. The ledger part lists every posting for the order
//! (one per posting key) with its entries and their sum, which must be zero,
//! and the order's balance per account.

use std::collections::BTreeMap;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{Owner, parse_id, views};
use crate::{AppState, error::AppError, extract::AppQuery, orders::views::Cursor};

const STATUSES: [&str; 9] = [
    "pending_payment",
    "paid",
    "picked_up",
    "out_for_delivery",
    "delivered",
    "completed",
    "cancelled",
    "refund_pending",
    "refunded",
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/orders", get(list))
        .route("/admin/orders/{id}", get(detail))
}

#[derive(Deserialize)]
struct ListQuery {
    status: Option<String>,
    store_id: Option<Uuid>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    from: Option<OffsetDateTime>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    to: Option<OffsetDateTime>,
    before: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize, sqlx::FromRow)]
struct OrderRow {
    order_id: Uuid,
    store_id: Uuid,
    status: String,
    items_total_kobo: i64,
    delivery_fee_kobo: i64,
    held: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    paid_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Serialize)]
struct OrderPage {
    items: Vec<OrderRow>,
    next_cursor: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    Owner(owner): Owner,
    AppQuery(q): AppQuery<ListQuery>,
) -> Result<Json<OrderPage>, AppError> {
    if let Some(s) = q.status.as_deref()
        && !STATUSES.contains(&s)
    {
        return Err(AppError::Validation(format!(
            "status must be one of {}",
            STATUSES.join(", ")
        )));
    }
    let limit = views::limit(q.limit)?;
    let cursor = q.before.as_deref().map(Cursor::decode).transpose()?;
    let mut items: Vec<OrderRow> = sqlx::query_as(
        "SELECT id AS order_id, store_id, status::text AS status, items_total_kobo, \
                delivery_fee_kobo, held_at IS NOT NULL AS held, paid_at, created_at \
         FROM orders \
         WHERE ($1::text IS NULL OR status::text = $1) \
           AND ($2::uuid IS NULL OR store_id = $2) \
           AND ($3::timestamptz IS NULL OR created_at >= $3) \
           AND ($4::timestamptz IS NULL OR created_at < $4) \
           AND ($5::timestamptz IS NULL OR (created_at, id) < ($5, $6)) \
         ORDER BY created_at DESC, id DESC LIMIT $7",
    )
    .bind(q.status.as_deref())
    .bind(q.store_id)
    .bind(q.from)
    .bind(q.to)
    .bind(cursor.as_ref().map(|c| c.0))
    .bind(cursor.as_ref().map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(&state.db)
    .await
    .context("search orders")?;
    let more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    let next_cursor = match (more, items.last()) {
        (true, Some(last)) => Some(Cursor(last.created_at, last.order_id).encode()?),
        _ => None,
    };
    let mut conn = state.db.acquire().await.context("acquire")?;
    views::record_view(&mut conn, owner.id, "orders", items.len()).await?;
    Ok(Json(OrderPage { items, next_cursor }))
}

#[derive(Serialize, sqlx::FromRow)]
struct OrderHead {
    order_id: Uuid,
    store_id: Uuid,
    status: String,
    items_total_kobo: i64,
    delivery_fee_kobo: i64,
    platform_cut_bps: i32,
    platform_cut_kobo: i64,
    collection_fee_share_kobo: i64,
    held: bool,
    hold_reason: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    paid_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    delivered_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    completed_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    cancelled_at: Option<OffsetDateTime>,
}

#[derive(Serialize, sqlx::FromRow)]
struct ItemRow {
    product_name: String,
    attributes: Value,
    unit_price_kobo: i64,
    quantity: i32,
}

#[derive(Serialize, sqlx::FromRow)]
struct PaymentRow {
    reference: String,
    method: String,
    provider: String,
    status: String,
    provider_flagged: bool,
}

#[derive(Serialize, sqlx::FromRow)]
struct EventRow {
    source: String,
    status: String,
    #[serde(with = "time::serde::rfc3339::option")]
    occurred_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    received_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
struct RefundRow {
    id: Uuid,
    kind: String,
    amount_kobo: i64,
    status: String,
}

#[derive(Serialize, sqlx::FromRow)]
struct PayoutRow {
    id: Uuid,
    status: String,
    amount_kobo: i64,
    fee_kobo: i64,
}

#[derive(Serialize, sqlx::FromRow)]
struct DisputeRow {
    id: Uuid,
    status: String,
}

#[derive(sqlx::FromRow)]
struct EntryRow {
    txn_id: Uuid,
    kind: Option<String>,
    reference: Option<String>,
    posted_at: Option<OffsetDateTime>,
    account: String,
    amount_kobo: i64,
    entry_kind: String,
}

#[derive(Serialize)]
struct EntryView {
    account: String,
    amount_kobo: i64,
    kind: String,
}

#[derive(Serialize)]
struct Posting {
    txn_id: Uuid,
    kind: Option<String>,
    #[serde(rename = "ref")]
    reference: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    posted_at: Option<OffsetDateTime>,
    entries: Vec<EntryView>,
    sum_kobo: i64,
}

#[derive(Serialize)]
struct Balance {
    account: String,
    kobo: i64,
}

#[derive(Serialize)]
struct Ledger {
    postings: Vec<Posting>,
    balances: Vec<Balance>,
    balance_kobo: i64,
}

/// The order's entries grouped by posting (one `txn_id` each, oldest first).
async fn ledger(db: &PgPool, order_id: Uuid) -> Result<Ledger, AppError> {
    let rows: Vec<EntryRow> = sqlx::query_as(
        "SELECT e.txn_id, p.kind, p.ref AS reference, p.posted_at, \
                e.account::text AS account, e.amount_kobo, e.kind AS entry_kind \
         FROM ledger_entries e LEFT JOIN ledger_postings p ON p.txn_id = e.txn_id \
         WHERE e.order_id = $1 ORDER BY e.id",
    )
    .bind(order_id)
    .fetch_all(db)
    .await
    .context("load ledger")?;

    let mut postings: Vec<Posting> = Vec::new();
    let mut balances: BTreeMap<String, i64> = BTreeMap::new();
    for r in rows {
        *balances.entry(r.account.clone()).or_default() += r.amount_kobo;
        if postings.last().is_none_or(|p| p.txn_id != r.txn_id) {
            postings.push(Posting {
                txn_id: r.txn_id,
                kind: r.kind.clone(),
                reference: r.reference.clone(),
                posted_at: r.posted_at,
                entries: Vec::new(),
                sum_kobo: 0,
            });
        }
        let p = postings.last_mut().expect("just pushed");
        p.sum_kobo += r.amount_kobo;
        p.entries.push(EntryView {
            account: r.account,
            amount_kobo: r.amount_kobo,
            kind: r.entry_kind,
        });
    }
    let balance_kobo = balances.values().sum();
    Ok(Ledger {
        postings,
        balances: balances
            .into_iter()
            .map(|(account, kobo)| Balance { account, kobo })
            .collect(),
        balance_kobo,
    })
}

#[derive(Serialize)]
struct Detail {
    order: OrderHead,
    items: Vec<ItemRow>,
    payment: PaymentRow,
    delivery_events: Vec<EventRow>,
    refund: Option<RefundRow>,
    payout: Option<PayoutRow>,
    dispute: Option<DisputeRow>,
    ledger: Ledger,
}

async fn detail(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<Json<Detail>, AppError> {
    let id = parse_id(&id)?;
    let db = &state.db;
    let order: OrderHead = sqlx::query_as(
        "SELECT id AS order_id, store_id, status::text AS status, items_total_kobo, \
                delivery_fee_kobo, platform_cut_bps, platform_cut_kobo, \
                collection_fee_share_kobo, held_at IS NOT NULL AS held, hold_reason, \
                created_at, paid_at, delivered_at, completed_at, cancelled_at \
         FROM orders WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await
    .context("load order")?
    .ok_or(AppError::NotFound)?;
    let items = sqlx::query_as(
        "SELECT product_name, attributes, unit_price_kobo, quantity FROM order_items \
         WHERE order_id = $1 ORDER BY product_name, variant_id",
    )
    .bind(id)
    .fetch_all(db)
    .await
    .context("load items")?;
    let payment = sqlx::query_as(
        "SELECT c.reference, c.method, c.provider, c.status::text AS status, \
                c.provider_flagged \
         FROM checkouts c JOIN orders o ON o.checkout_id = c.id WHERE o.id = $1",
    )
    .bind(id)
    .fetch_one(db)
    .await
    .context("load payment")?;
    let delivery_events = sqlx::query_as(
        "SELECT source, status, occurred_at, received_at FROM delivery_events \
         WHERE order_id = $1 ORDER BY received_at, id",
    )
    .bind(id)
    .fetch_all(db)
    .await
    .context("load delivery events")?;
    let refund = sqlx::query_as(
        "SELECT id, kind, amount_kobo, status::text AS status FROM refunds WHERE order_id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await
    .context("load refund")?;
    let payout = sqlx::query_as(
        "SELECT id, status::text AS status, amount_kobo, fee_kobo FROM payouts \
         WHERE order_id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await
    .context("load payout")?;
    let dispute = sqlx::query_as("SELECT id, status FROM disputes WHERE order_id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
        .context("load dispute")?;
    let ledger = ledger(db, id).await?;

    let mut conn = db.acquire().await.context("acquire")?;
    views::record_view(&mut conn, owner.id, "order_detail", 1).await?;
    Ok(Json(Detail {
        order,
        items,
        payment,
        delivery_events,
        refund,
        payout,
        dispute,
        ledger,
    }))
}
