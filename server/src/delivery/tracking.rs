//! `GET /orders/{id}/tracking`: a fixed timeline, who to call, and (for the
//! buyer) the handover code. Mostly a question of who sees what, and when.
//!
//! - **Audience.** The order's buyer, or the owner of its store (never for a
//!   `pending_payment` order). Everyone else, staff included, and any foreign or
//!   malformed id, gets 404.
//! - **Every gate is read from the database on each request** (status, delivery
//!   method, who is asking). Nothing is cached; the route sets `no-store`.
//! - **Buyer** sees the courier's name and phone (courier orders) or the store's
//!   name and business phone (self-delivery, once `out_for_delivery`), never
//!   after delivery; the courier's tracking link when it is https; the handover
//!   code only while `out_for_delivery`.
//! - **Seller** sees `delivery_area` only, except on a self-delivery order that
//!   is `out_for_delivery`: then the buyer's recipient name, phone, address line
//!   and landmark, decrypted from the order's address snapshot. A courier order
//!   never shows the seller any buyer contact, and the seller never receives the
//!   courier's phone.
//! - **Audit.** Revealing contact writes `order.contact_revealed` once per
//!   (order, viewer), not per view, and fails the request if it cannot be
//!   written. The detail names the kind of reveal only, never a value.
//!
//! Each reveal sits behind one function ([`reveal_buyer_contact`],
//! [`reveal_delivery_contact`]) so that a proxy number can replace the real
//! one later.

use anyhow::Context;
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::{addresses::DeliveryAddress, area::AreaCouncil, promise};
use crate::{AppState, audit, crypto::FieldCipher, error::AppError, stores::service};

/// Audit action for a contact reveal.
pub const CONTACT_REVEALED: &str = "order.contact_revealed";
/// Shown with the handover code.
pub const HANDOVER_NOTE: &str = "Give this only to the person handing you the parcel";

const STEPS: [(&str, &str); 6] = [
    ("paid", "Paid"),
    ("preparing", "Seller preparing"),
    ("picked_up", "Picked up"),
    ("out_for_delivery", "Out for delivery"),
    ("delivered", "Delivered"),
    ("confirmed", "Confirmed"),
];

#[derive(Debug, Serialize)]
pub struct Step {
    pub key: &'static str,
    pub label: &'static str,
    #[serde(with = "time::serde::rfc3339::option")]
    pub at: Option<OffsetDateTime>,
    pub current: bool,
}

/// Who the buyer can call. Never the buyer's or seller's personal number.
#[derive(Serialize)]
pub struct DeliveryContact {
    pub name: Option<String>,
    pub phone: Option<String>,
}

impl std::fmt::Debug for DeliveryContact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeliveryContact(<redacted>)")
    }
}

/// What a self-delivering seller sees of the buyer while out for delivery.
#[derive(Serialize)]
pub struct BuyerContact {
    pub recipient_name: String,
    pub phone: String,
    pub address_line: String,
    pub landmark: Option<String>,
}

impl std::fmt::Debug for BuyerContact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BuyerContact(<redacted>)")
    }
}

#[derive(Serialize)]
pub struct TrackingView {
    pub order_id: Uuid,
    /// `YYYY-MM-DD`, Africa/Lagos. None on an order from before delivery existed.
    pub promised_date: Option<String>,
    pub status: String,
    pub steps: Vec<Step>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_note: Option<String>,
    /// Buyer only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<DeliveryContact>,
    /// Buyer only, https only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracking_url: Option<String>,
    /// Buyer only, self-delivery, while out for delivery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handover_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handover_note: Option<&'static str>,
    /// Seller only, and only when the order is held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held: Option<bool>,
    /// Seller only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_area: Option<AreaCouncil>,
    /// Seller only, self-delivery, while out for delivery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buyer_contact: Option<BuyerContact>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Viewer {
    Buyer,
    Seller,
}

#[derive(sqlx::FromRow)]
pub(crate) struct Facts {
    buyer_id: Uuid,
    store_id: Uuid,
    store_name: String,
    status: String,
    delivery_method: Option<String>,
    promised_date: Option<Date>,
    held: bool,
    delivery_area: Option<AreaCouncil>,
    paid_at: Option<OffsetDateTime>,
    ready_at: Option<OffsetDateTime>,
    picked_up_at: Option<OffsetDateTime>,
    out_for_delivery_at: Option<OffsetDateTime>,
    delivered_at: Option<OffsetDateTime>,
    completed_at: Option<OffsetDateTime>,
    delivery_address_enc: Option<Vec<u8>>,
    handover_code_enc: Option<Vec<u8>>,
}

impl Facts {
    fn is_self(&self) -> bool {
        self.delivery_method.as_deref() == Some("self")
    }

    fn is_courier(&self) -> bool {
        self.delivery_method.as_deref() == Some("courier")
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct ShipmentFacts {
    status: String,
    tracking_url: Option<String>,
    courier_name: Option<String>,
    courier_phone_enc: Option<Vec<u8>>,
}

impl ShipmentFacts {
    /// A booked courier, not one whose booking is pending, failed or cancelled.
    fn is_live(&self) -> bool {
        matches!(
            self.status.as_str(),
            "booked" | "confirmed" | "picked_up" | "in_transit" | "completed"
        )
    }
}

/// The tracking view of order `order_id` for `viewer`; 404 for anyone who is
/// not its buyer or its store's owner.
pub async fn get(state: &AppState, viewer: Uuid, order_id: Uuid) -> Result<TrackingView, AppError> {
    let facts: Facts = sqlx::query_as(
        "SELECT o.buyer_id, o.store_id, s.name AS store_name, o.status::text AS status, \
                o.delivery_method::text AS delivery_method, o.promised_date, \
                (o.held_at IS NOT NULL) AS held, o.delivery_area, \
                COALESCE(o.paid_at, c.paid_at) AS paid_at, o.ready_at, o.picked_up_at, \
                o.out_for_delivery_at, o.delivered_at, o.completed_at, \
                o.delivery_address_enc, o.handover_code_enc \
         FROM orders o \
         JOIN checkouts c ON c.id = o.checkout_id \
         JOIN stores s ON s.id = o.store_id \
         WHERE o.id = $1 AND (o.buyer_id = $2 \
               OR (s.owner_id = $2 AND o.status <> 'pending_payment'))",
    )
    .bind(order_id)
    .bind(viewer)
    .fetch_optional(&state.db)
    .await
    .context("load tracking order")?
    .ok_or(AppError::NotFound)?;
    let who = if facts.buyer_id == viewer {
        Viewer::Buyer
    } else {
        Viewer::Seller
    };

    let mut view = TrackingView {
        order_id,
        promised_date: facts.promised_date.map(|d| d.to_string()),
        status: shown_status(&facts.status, who),
        steps: steps(&facts),
        delay_note: delay_note(&facts, OffsetDateTime::now_utc()),
        contact: None,
        tracking_url: None,
        handover_code: None,
        handover_note: None,
        held: None,
        delivery_area: None,
        buyer_contact: None,
    };

    let kind = match who {
        Viewer::Buyer => {
            let shipment = if facts.is_courier() {
                load_shipment(&state.db, order_id).await?
            } else {
                None
            };
            view.contact =
                reveal_delivery_contact(&state.db, &state.cipher, &facts, shipment.as_ref())
                    .await?;
            view.tracking_url = tracking_url(&facts, shipment.as_ref());
            if handover_code_visible(&facts)
                && let Some(enc) = &facts.handover_code_enc
            {
                view.handover_code = Some(state.cipher.decrypt(enc)?);
                view.handover_note = Some(HANDOVER_NOTE);
            }
            view.contact.is_some().then_some("delivery_contact")
        }
        Viewer::Seller => {
            view.delivery_area = facts.delivery_area;
            view.held = facts.held.then_some(true);
            view.buyer_contact = reveal_buyer_contact(&state.cipher, &facts)?;
            view.buyer_contact.is_some().then_some("buyer_contact")
        }
    };
    if let Some(kind) = kind {
        // Fail closed: nothing is returned that could not be audited.
        audit_reveal_once(&state.db, viewer, order_id, kind).await?;
    }
    Ok(view)
}

/// Sellers see refund mechanics as a plain `cancelled`, as in the order views.
fn shown_status(status: &str, who: Viewer) -> String {
    match (who, status) {
        (Viewer::Seller, "refund_pending" | "refunded") => "cancelled".into(),
        _ => status.into(),
    }
}

fn steps(f: &Facts) -> Vec<Step> {
    let current = match f.status.as_str() {
        "paid" => Some("preparing"),
        "picked_up" => Some("picked_up"),
        "out_for_delivery" => Some("out_for_delivery"),
        "delivered" => Some("delivered"),
        "completed" => Some("confirmed"),
        _ => None,
    };
    STEPS
        .iter()
        .map(|&(key, label)| Step {
            key,
            label,
            at: match key {
                "paid" => f.paid_at,
                // Only a courier order has a ready step; a self order is never "ready".
                "preparing" if f.is_courier() => f.ready_at,
                "preparing" => None,
                "picked_up" => f.picked_up_at,
                "out_for_delivery" => f.out_for_delivery_at,
                "delivered" => f.delivered_at,
                _ => f.completed_at,
            },
            current: current == Some(key),
        })
        .collect()
}

/// Derived, never stored: today (Africa/Lagos) is past the promised date and
/// the order is not delivered or later. Changes no state.
fn delay_note(f: &Facts, now: OffsetDateTime) -> Option<String> {
    let promised = f.promised_date?;
    let open = matches!(f.status.as_str(), "paid" | "picked_up" | "out_for_delivery");
    (open && promise::lagos_date(now) > promised)
        .then(|| format!("This order is running late. It was promised for {promised}."))
}

fn handover_code_visible(f: &Facts) -> bool {
    f.is_self() && f.status == "out_for_delivery"
}

/// https only, and only for a buyer whose order is still live.
fn tracking_url(f: &Facts, shipment: Option<&ShipmentFacts>) -> Option<String> {
    if matches!(
        f.status.as_str(),
        "cancelled" | "refund_pending" | "refunded"
    ) {
        return None;
    }
    let s = shipment.filter(|s| s.is_live())?;
    s.tracking_url
        .as_deref()
        .filter(|u| u.starts_with("https://"))
        .map(str::to_owned)
}

async fn load_shipment(db: &PgPool, order_id: Uuid) -> Result<Option<ShipmentFacts>, AppError> {
    Ok(sqlx::query_as(
        "SELECT status::text AS status, tracking_url, courier_name, courier_phone_enc \
         FROM shipments WHERE order_id = $1",
    )
    .bind(order_id)
    .fetch_optional(db)
    .await
    .context("load tracking shipment")?)
}

/// The one place the buyer's call button is decided. Courier orders: the
/// courier's name and phone, from before pickup until delivery. Self-delivery:
/// the store's name and business phone, only once out for delivery. Nothing
/// after delivery (or on a cancelled or refunded order). A proxy number would
/// replace the phone here.
pub(crate) async fn reveal_delivery_contact(
    db: &PgPool,
    cipher: &FieldCipher,
    f: &Facts,
    shipment: Option<&ShipmentFacts>,
) -> Result<Option<DeliveryContact>, AppError> {
    if f.is_courier() {
        if !matches!(f.status.as_str(), "paid" | "picked_up" | "out_for_delivery") {
            return Ok(None);
        }
        let Some(s) = shipment.filter(|s| s.is_live()) else {
            return Ok(None);
        };
        let phone = s
            .courier_phone_enc
            .as_deref()
            .map(|b| cipher.decrypt(b))
            .transpose()?;
        if s.courier_name.is_none() && phone.is_none() {
            return Ok(None);
        }
        return Ok(Some(DeliveryContact {
            name: s.courier_name.clone(),
            phone,
        }));
    }
    if f.is_self() && f.status == "out_for_delivery" {
        let phone = match service::delivery_profile(db, cipher, f.store_id).await {
            Ok(p) => p.business_phone,
            // A deleted store: nothing to call.
            Err(AppError::NotFound) => None,
            Err(e) => return Err(e),
        };
        if phone.is_none() {
            return Ok(None);
        }
        return Ok(Some(DeliveryContact {
            name: Some(f.store_name.clone()),
            phone,
        }));
    }
    Ok(None)
}

/// The one place the buyer's contact reaches a seller. Only a self-delivery
/// order that is out for delivery: decrypted from the order's address snapshot.
/// A proxy number would replace the phone here.
pub(crate) fn reveal_buyer_contact(
    cipher: &FieldCipher,
    f: &Facts,
) -> Result<Option<BuyerContact>, AppError> {
    if !(f.is_self() && f.status == "out_for_delivery") {
        return Ok(None);
    }
    let Some(enc) = &f.delivery_address_enc else {
        return Ok(None);
    };
    let plain = cipher.decrypt(enc)?;
    // The parse error would quote the data: keep it out.
    let address: DeliveryAddress = serde_json::from_str(&plain)
        .map_err(|_| anyhow::anyhow!("corrupt delivery address snapshot"))?;
    Ok(Some(BuyerContact {
        recipient_name: address.recipient_name,
        phone: address.phone,
        address_line: address.address_line,
        landmark: address.landmark,
    }))
}

/// One `order.contact_revealed` row per (order, viewer), however often they
/// look. An advisory lock makes two first views at once write one row.
async fn audit_reveal_once(
    db: &PgPool,
    viewer: Uuid,
    order_id: Uuid,
    kind: &str,
) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("{CONTACT_REVEALED}:{order_id}:{viewer}"))
        .execute(&mut *tx)
        .await
        .context("lock reveal audit")?;
    let seen: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM audit_log WHERE action = $1 \
         AND subject_type = 'order' AND subject_id = $2 AND actor_id = $3)",
    )
    .bind(CONTACT_REVEALED)
    .bind(order_id)
    .bind(viewer)
    .fetch_one(&mut *tx)
    .await
    .context("check reveal audit")?;
    if !seen {
        audit::record(
            &mut tx,
            Some(viewer),
            CONTACT_REVEALED,
            "order",
            Some(order_id),
            json!({ "kind": kind }),
        )
        .await?;
    }
    tx.commit().await.context("commit")?;
    Ok(())
}
