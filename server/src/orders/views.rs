//! Read models for orders. Two audiences, two structs, each an explicit
//! allowlist of fields (never serialise a database row):
//!
//! - [`BuyerOrderView`]: items, totals, status, timestamps, held flag. No
//!   payout data, and no platform cut (as at checkout).
//! - [`SellerOrderView`]: the same plus the platform cut, the buyer's display
//!   name only (no id, phone or address), the expected payout and the
//!   payout's status, and who in the store's team did each step (`done_by`,
//!   from the audit log). Refund mechanics are hidden behind a coarse `cancelled`
//!   status. The expected payout uses the fee schedule snapshotted on the
//!   order when it was paid (the live one only for orders paid before the
//!   snapshot existed), is never below 0, and is left out once the order is
//!   cancelled or refunded.
//! - Both views carry `refunded_kobo` (the order's refund amount, full or
//!   partial) when the order has a refund. After a partial refund the
//!   order's `platform_cut_kobo` is the cut on the retained items.
//!
//! Every query puts the principal in the SQL, so a missing order and someone
//! else's order are indistinguishable (both `NotFound`).
//!
//! **Who is the seller.** The store's owner, a manager, store-wide staff, or
//! staff of the order's branch (`stores::access`); staff of another branch get
//! `NotFound` for the order and do not see it in the store's list.
//!
//! **Staff reads.** Owner and Support may read any paid order through
//! [`get_for_viewer`]; when the order is not their own (as buyer or store
//! member), the read writes an `admin.viewed` audit row (`view: "order"`), like
//! every admin list. Support gets the seller view without the buyer's display
//! name; the Owner keeps it (pending DPO review).

use std::collections::{BTreeMap, HashMap};

use anyhow::Context;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{
    audit,
    error::AppError,
    money::{self, FeeSchedule},
    settings,
    stores::access::{self, Action, covers_branch_sql},
};

pub const DEFAULT_LIMIT: i64 = 20;
pub const MAX_LIMIT: i64 = 100;

#[derive(Debug, Serialize)]
pub struct ItemView {
    pub product_name: String,
    pub attributes: Value,
    pub unit_price_kobo: i64,
    pub quantity: i32,
}

#[derive(Debug, Serialize)]
pub struct PayoutView {
    pub status: String,
}

/// The member who did a step: their id and display name, nothing else.
#[derive(Debug, Clone, Serialize)]
pub struct DoneBy {
    pub user_id: Uuid,
    pub display_name: String,
}

/// Seller steps and the audit action that names who did each.
const STEP_ACTIONS: [(&str, &str); 4] = [
    ("order.ready", "ready"),
    ("order.seller_picked_up", "picked_up"),
    ("order.seller_out_for_delivery", "out_for_delivery"),
    ("order.delivered", "delivered"),
];

#[derive(Debug, Serialize)]
pub struct DisputeSummary {
    pub id: Uuid,
    pub status: String,
}

/// Fields common to both audiences.
#[derive(Debug, Serialize)]
pub struct OrderCore {
    pub id: Uuid,
    pub store_id: Uuid,
    pub status: String,
    pub items: Vec<ItemView>,
    pub items_total_kobo: i64,
    pub delivery_fee_kobo: i64,
    pub total_kobo: i64,
    pub held: bool,
    /// Left out when the order has no dispute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispute: Option<DisputeSummary>,
    /// What went back to the buyer: the order's refund (full or partial).
    /// Left out when the order has no refund.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refunded_kobo: Option<i64>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub paid_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub ship_by: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub delivered_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub release_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
pub struct BuyerOrderView {
    #[serde(flatten)]
    pub order: OrderCore,
}

#[derive(Debug, Serialize)]
pub struct SellerOrderView {
    #[serde(flatten)]
    pub order: OrderCore,
    pub platform_cut_kobo: i64,
    /// Left out for Support (see the module docs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buyer_display_name: Option<String>,
    /// Left out once the order is cancelled or refunded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_payout_kobo: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payout: Option<PayoutView>,
    /// Step (`ready`, `picked_up`, `out_for_delivery`, `delivered`) to the member
    /// who did it, where the audit names one. Left out when none does.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub done_by: BTreeMap<&'static str, DoneBy>,
}

/// Staff reading an order through [`get_for_viewer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaffRole {
    Owner,
    Support,
}

/// Which view `build` makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Audience {
    Buyer,
    /// The seller view; `false` leaves out the buyer's display name.
    Seller {
        buyer_name: bool,
    },
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OrderView {
    Buyer(BuyerOrderView),
    Seller(SellerOrderView),
}

impl OrderView {
    fn key(&self) -> (OffsetDateTime, Uuid) {
        let o = match self {
            OrderView::Buyer(b) => &b.order,
            OrderView::Seller(s) => &s.order,
        };
        (o.created_at, o.id)
    }
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Pass as `?before=` for the next page; null on the last page.
    pub next_cursor: Option<String>,
}

/// Keyset cursor `(created_at, id)`, opaque to clients.
pub(crate) struct Cursor(pub(crate) OffsetDateTime, pub(crate) Uuid);

impl Cursor {
    pub(crate) fn encode(&self) -> Result<String, AppError> {
        let ts = self.0.format(&Rfc3339).context("format cursor")?;
        Ok(URL_SAFE_NO_PAD.encode(format!("{ts}|{}", self.1)))
    }

    pub(crate) fn decode(s: &str) -> Result<Self, AppError> {
        let bad = || AppError::Validation("before is not a valid cursor".into());
        let raw = URL_SAFE_NO_PAD.decode(s).map_err(|_| bad())?;
        let raw = String::from_utf8(raw).map_err(|_| bad())?;
        let (ts, id) = raw.split_once('|').ok_or_else(bad)?;
        Ok(Cursor(
            OffsetDateTime::parse(ts, &Rfc3339).map_err(|_| bad())?,
            id.parse().map_err(|_| bad())?,
        ))
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    store_id: Uuid,
    buyer_id: Uuid,
    /// The viewer may act on this order as a member of its store.
    viewer_member: bool,
    status: String,
    items_total_kobo: i64,
    delivery_fee_kobo: i64,
    self_delivery: bool,
    platform_cut_kobo: i64,
    collection_fee_share_kobo: i64,
    held: bool,
    dispute_id: Option<Uuid>,
    dispute_status: Option<String>,
    paid_at: Option<OffsetDateTime>,
    ship_by: Option<OffsetDateTime>,
    delivered_at: Option<OffsetDateTime>,
    release_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    buyer_display_name: String,
    payout_status: Option<String>,
    payout_amount_kobo: Option<i64>,
    /// The order's refund amount (full or partial; one refund per order).
    refunded_kobo: Option<i64>,
    fee_small_kobo: Option<i64>,
    fee_large_kobo: Option<i64>,
    fee_threshold_kobo: Option<i64>,
    stamp_duty_kobo: Option<i64>,
    stamp_duty_threshold_kobo: Option<i64>,
}

impl Row {
    /// The payout fee schedule snapshotted when the order was paid, if any.
    fn fee_snapshot(&self) -> Option<FeeSchedule> {
        Some(FeeSchedule {
            small_fee: self.fee_small_kobo?,
            large_fee: self.fee_large_kobo?,
            threshold: self.fee_threshold_kobo?,
            stamp_duty_kobo: self.stamp_duty_kobo?,
            stamp_duty_threshold: self.stamp_duty_threshold_kobo?,
        })
    }

    /// What the seller should expect to be paid (see the module docs): the
    /// payout's own amount once released, else the sum release makes. None
    /// once the order is cancelled or refunded. A partially refunded order
    /// (`completed`) shows its payout's amount, which is on the retained items
    /// only: the estimate from the order's own totals would overstate it.
    fn expected_payout(&self, live: Option<&FeeSchedule>) -> Option<i64> {
        if matches!(
            self.status.as_str(),
            "cancelled" | "refund_pending" | "refunded"
        ) {
            return None;
        }
        if let Some(amount) = self.payout_amount_kobo {
            return Some(amount.max(0));
        }
        let snapshot = self.fee_snapshot();
        let schedule = snapshot.as_ref().or(live)?;
        // A self-delivery fee is paid to the seller; a courier fee is not.
        let seller_delivery = if self.self_delivery {
            self.delivery_fee_kobo
        } else {
            0
        };
        let before_fee = self
            .items_total_kobo
            .saturating_sub(self.platform_cut_kobo)
            .saturating_sub(self.collection_fee_share_kobo)
            .saturating_add(seller_delivery);
        Some(
            before_fee
                .saturating_sub(money::payout_fee_kobo(before_fee, schedule))
                .max(0),
        )
    }
}

/// The order query; `viewer_member` is an SQL boolean over `s` and `o`.
fn select(viewer_member: &str) -> String {
    format!(
        "SELECT o.id, o.store_id, o.buyer_id, ({viewer_member}) AS viewer_member, \
        o.status::text AS status, \
        o.items_total_kobo, o.delivery_fee_kobo, COALESCE(o.delivery_method::text = 'self', false) AS self_delivery, \
        o.platform_cut_kobo, \
        o.collection_fee_share_kobo, (o.held_at IS NOT NULL) AS held, \
        d.id AS dispute_id, d.status AS dispute_status, \
        COALESCE(o.paid_at, c.paid_at) AS paid_at, o.ship_by, o.delivered_at, o.release_at, \
        o.created_at, u.display_name AS buyer_display_name, \
        p.status::text AS payout_status, p.amount_kobo AS payout_amount_kobo, \
        rf.amount_kobo AS refunded_kobo, \
        o.fee_small_kobo, o.fee_large_kobo, o.fee_threshold_kobo, \
        o.stamp_duty_kobo, o.stamp_duty_threshold_kobo \
     FROM orders o \
     JOIN checkouts c ON c.id = o.checkout_id \
     JOIN users u ON u.id = o.buyer_id \
     JOIN stores s ON s.id = o.store_id \
     LEFT JOIN payouts p ON p.order_id = o.id \
     LEFT JOIN disputes d ON d.order_id = o.id \
     LEFT JOIN refunds rf ON rf.order_id = o.id"
    )
}

/// Who did each seller step of the orders, from the audit log (first entry per step).
async fn done_by_for(
    db: &PgPool,
    ids: &[Uuid],
) -> Result<HashMap<Uuid, BTreeMap<&'static str, DoneBy>>, AppError> {
    let actions: Vec<&str> = STEP_ACTIONS.iter().map(|(a, _)| *a).collect();
    let rows: Vec<(Uuid, String, Uuid, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (a.subject_id, a.action) a.subject_id, a.action, a.actor_id, \
                u.display_name \
         FROM audit_log a JOIN users u ON u.id = a.actor_id \
         WHERE a.subject_type = 'order' AND a.subject_id = ANY($1) AND a.action = ANY($2) \
         ORDER BY a.subject_id, a.action, a.id",
    )
    .bind(ids)
    .bind(&actions)
    .fetch_all(db)
    .await
    .context("load step actors")?;
    let mut map: HashMap<Uuid, BTreeMap<&'static str, DoneBy>> = HashMap::new();
    for (order_id, action, user_id, display_name) in rows {
        if let Some((_, step)) = STEP_ACTIONS.iter().find(|(a, _)| *a == action) {
            map.entry(order_id).or_default().insert(
                step,
                DoneBy {
                    user_id,
                    display_name,
                },
            );
        }
    }
    Ok(map)
}

async fn items_for(db: &PgPool, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<ItemView>>, AppError> {
    let rows: Vec<(Uuid, String, Value, i64, i32)> = sqlx::query_as(
        "SELECT order_id, product_name, attributes, unit_price_kobo, quantity \
         FROM order_items WHERE order_id = ANY($1) ORDER BY order_id, product_name, variant_id",
    )
    .bind(ids)
    .fetch_all(db)
    .await
    .context("load order items")?;
    let mut map: HashMap<Uuid, Vec<ItemView>> = HashMap::new();
    for (order_id, product_name, attributes, unit_price_kobo, quantity) in rows {
        map.entry(order_id).or_default().push(ItemView {
            product_name,
            attributes,
            unit_price_kobo,
            quantity,
        });
    }
    Ok(map)
}

fn core(r: &Row, items: Vec<ItemView>, seller: bool) -> Result<OrderCore, AppError> {
    let total = r
        .items_total_kobo
        .checked_add(r.delivery_fee_kobo)
        .context("order total overflow")?;
    // Sellers get a coarse status: refund mechanics stay internal.
    let status = match (seller, r.status.as_str()) {
        (true, "refund_pending" | "refunded") => "cancelled".to_string(),
        _ => r.status.clone(),
    };
    Ok(OrderCore {
        id: r.id,
        store_id: r.store_id,
        status,
        items,
        items_total_kobo: r.items_total_kobo,
        delivery_fee_kobo: r.delivery_fee_kobo,
        total_kobo: total,
        held: r.held,
        dispute: r
            .dispute_id
            .zip(r.dispute_status.clone())
            .map(|(id, status)| DisputeSummary { id, status }),
        refunded_kobo: r.refunded_kobo,
        paid_at: r.paid_at,
        ship_by: r.ship_by,
        delivered_at: r.delivered_at,
        release_at: r.release_at,
        created_at: r.created_at,
    })
}

async fn build(
    db: &PgPool,
    rows: Vec<Row>,
    audience: Audience,
) -> Result<Vec<OrderView>, AppError> {
    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let mut items = items_for(db, &ids).await?;
    let Audience::Seller { buyer_name } = audience else {
        return rows
            .into_iter()
            .map(|r| {
                let order = core(&r, items.remove(&r.id).unwrap_or_default(), false)?;
                Ok(OrderView::Buyer(BuyerOrderView { order }))
            })
            .collect();
    };
    // The live schedule only for orders paid before the snapshot existed.
    let live = if rows.iter().any(|r| r.fee_snapshot().is_none()) {
        Some(settings::load(db).await?.fee_schedule())
    } else {
        None
    };
    let mut done_by = done_by_for(db, &ids).await?;
    rows.into_iter()
        .map(|r| {
            let order = core(&r, items.remove(&r.id).unwrap_or_default(), true)?;
            Ok(OrderView::Seller(SellerOrderView {
                order,
                platform_cut_kobo: r.platform_cut_kobo,
                expected_payout_kobo: r.expected_payout(live.as_ref()),
                buyer_display_name: buyer_name.then_some(r.buyer_display_name),
                payout: r.payout_status.map(|status| PayoutView { status }),
                done_by: done_by.remove(&r.id).unwrap_or_default(),
            }))
        })
        .collect()
}

async fn page(
    db: &PgPool,
    mut rows: Vec<Row>,
    limit: i64,
    audience: Audience,
) -> Result<Page<OrderView>, AppError> {
    let more = rows.len() as i64 > limit;
    rows.truncate(usize::try_from(limit).unwrap_or(0));
    let items = build(db, rows, audience).await?;
    let next_cursor = match (more, items.last()) {
        (true, Some(last)) => {
            let (ts, id) = last.key();
            Some(Cursor(ts, id).encode()?)
        }
        _ => None,
    };
    Ok(Page { items, next_cursor })
}

/// The buyer's own orders, newest first, keyset-paged on `(created_at, id)`.
pub async fn list_for_buyer(
    db: &PgPool,
    buyer: Uuid,
    limit: i64,
    before: Option<&str>,
) -> Result<Page<OrderView>, AppError> {
    let cur = before.map(Cursor::decode).transpose()?;
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "{} WHERE o.buyer_id = $1 \
         AND ($2::timestamptz IS NULL OR (o.created_at, o.id) < ($2, $3::uuid)) \
         ORDER BY o.created_at DESC, o.id DESC LIMIT $4",
        select("false")
    ))
    .bind(buyer)
    .bind(cur.as_ref().map(|c| c.0))
    .bind(cur.as_ref().map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(db)
    .await
    .context("list buyer orders")?;
    page(db, rows, limit, Audience::Buyer).await
}

/// A store's orders, for a member of it: all of them, or only their branch's for
/// staff tied to one. A store that is missing, deleted or not theirs yields
/// `NotFound`.
pub async fn list_for_store(
    db: &PgPool,
    viewer: Uuid,
    store_id: Uuid,
    limit: i64,
    before: Option<&str>,
) -> Result<Page<OrderView>, AppError> {
    let cur = before.map(Cursor::decode).transpose()?;
    let mut conn = db.acquire().await.context("acquire")?;
    access::require(&mut conn, store_id, viewer, Action::ViewStore).await?;
    drop(conn);
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "{} WHERE o.store_id = $1 AND {} \
         AND o.status <> 'pending_payment' \
         AND ($2::timestamptz IS NULL OR (o.created_at, o.id) < ($2, $3::uuid)) \
         ORDER BY o.created_at DESC, o.id DESC LIMIT $4",
        select("true"),
        covers_branch_sql("$5", "o.branch_id")
    ))
    .bind(store_id)
    .bind(cur.as_ref().map(|c| c.0))
    .bind(cur.as_ref().map(|c| c.1))
    .bind(limit + 1)
    .bind(viewer)
    .fetch_all(db)
    .await
    .context("list store orders")?;
    page(db, rows, limit, Audience::Seller { buyer_name: true }).await
}

/// One order for `viewer`: the buyer sees the buyer view; a member who may act on
/// it (see the module docs) and owner/support staff (`staff`) see the seller view. The principal is part of
/// the query, so a missing order and someone else's order are both
/// `NotFound`. A staff read of an order that is not the viewer's own is
/// audited (`admin.viewed`, written before the response, so a failure fails
/// the read), and Support gets no buyer display name (see the module docs).
pub async fn get_for_viewer(
    db: &PgPool,
    viewer: Uuid,
    staff: Option<StaffRole>,
    order_id: Uuid,
) -> Result<OrderView, AppError> {
    let member = covers_branch_sql("$2", "o.branch_id");
    let row: Option<Row> = sqlx::query_as(&format!(
        "{} WHERE o.id = $1 AND (o.buyer_id = $2 OR \
         (({member} OR $3) AND o.status <> 'pending_payment'))",
        select(&member)
    ))
    .bind(order_id)
    .bind(viewer)
    .bind(staff.is_some())
    .fetch_optional(db)
    .await
    .context("load order")?;
    let row = row.ok_or(AppError::NotFound)?;
    let audience = if row.buyer_id == viewer {
        Audience::Buyer
    } else if row.viewer_member {
        Audience::Seller { buyer_name: true }
    } else {
        // Only staff get here: the query admits no one else.
        let mut conn = db.acquire().await.context("acquire")?;
        audit::record(
            &mut conn,
            Some(viewer),
            "admin.viewed",
            "order",
            Some(order_id),
            json!({ "view": "order" }),
        )
        .await?;
        Audience::Seller {
            buyer_name: staff == Some(StaffRole::Owner),
        }
    };
    build(db, vec![row], audience)
        .await?
        .pop()
        .ok_or(AppError::NotFound)
}
