//! The courier provider layer. Delivery logic talks only to [`CourierProvider`];
//! Shipbubble is the production implementation and [`fake::FakeCourier`] the
//! test double.
//!
//! Data protection: address, phone and email fields are personal data. Types
//! holding them have a hand-written `Debug` that redacts them, and
//! [`CourierError`] never prints its inner detail, so none of it reaches logs.

pub mod fake;
pub mod shipbubble;

use std::fmt;

use async_trait::async_trait;
use axum::http::HeaderMap;
use time::{Date, OffsetDateTime};

use crate::delivery::area::AreaCouncil;
use crate::error::AppError;

/// Dimension (cm) used when a product has none recorded.
const DEFAULT_DIM_CM: i32 = 10;
/// Upper bound for the stacked parcel height, so huge quantities cannot overflow.
const MAX_HEIGHT_CM: i32 = 300;

/// A sender or receiver for address validation. Personal data.
#[derive(Clone)]
pub struct ContactAddress {
    pub name: String,
    pub email: Option<String>,
    pub phone: String,
    pub address_line: String,
    pub area: AreaCouncil,
}

impl fmt::Debug for ContactAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContactAddress")
            .field("area", &self.area)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParcelItem {
    pub name: String,
    pub unit_weight_grams: i32,
    pub unit_amount_kobo: i64,
    pub quantity: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parcel {
    pub items: Vec<ParcelItem>,
    pub length_cm: i32,
    pub width_cm: i32,
    pub height_cm: i32,
    pub category_id: i64,
}

/// A product's shipping-relevant facts, as input to [`parcel_for`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductDims {
    pub name: String,
    pub weight_grams: i32,
    pub unit_amount_kobo: i64,
    pub length_cm: Option<i32>,
    pub width_cm: Option<i32>,
    pub height_cm: Option<i32>,
}

/// Builds a parcel from `(product, quantity)` pairs: the largest length and
/// width, and the heights stacked (height x quantity, summed, capped at 300 cm).
/// Missing dimensions default to 10 cm.
pub fn parcel_for(items: &[(ProductDims, i32)], category_id: i64) -> Parcel {
    let mut length = DEFAULT_DIM_CM;
    let mut width = DEFAULT_DIM_CM;
    let mut height: i32 = 0;
    let mut out = Vec::with_capacity(items.len());
    for (p, qty) in items {
        let qty = (*qty).max(0);
        length = length.max(p.length_cm.unwrap_or(DEFAULT_DIM_CM));
        width = width.max(p.width_cm.unwrap_or(DEFAULT_DIM_CM));
        let h = p.height_cm.unwrap_or(DEFAULT_DIM_CM).max(0);
        height = height
            .saturating_add(h.saturating_mul(qty))
            .min(MAX_HEIGHT_CM);
        out.push(ParcelItem {
            name: p.name.clone(),
            unit_weight_grams: p.weight_grams,
            unit_amount_kobo: p.unit_amount_kobo,
            quantity: qty,
        });
    }
    Parcel {
        items: out,
        length_cm: length,
        width_cm: width,
        height_cm: height.max(1),
        category_id,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rate {
    pub courier_id: String,
    pub service_code: String,
    pub courier_name: String,
    /// The courier's rate card amount.
    pub amount_kobo: i64,
    /// What the courier charges in total.
    pub charged_kobo: i64,
    pub delivery_eta: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateQuote {
    pub request_token: String,
    pub rates: Vec<Rate>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Booking {
    pub provider_order_id: String,
    pub tracking_url: Option<String>,
    pub courier_name: String,
    pub courier_phone: Option<String>,
    pub charged_kobo: Option<i64>,
}

impl fmt::Debug for Booking {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Booking")
            .field("provider_order_id", &self.provider_order_id)
            .field("courier_name", &self.courier_name)
            .field("charged_kobo", &self.charged_kobo)
            .finish_non_exhaustive()
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipmentState {
    Pending,
    Confirmed,
    PickedUp,
    InTransit,
    Completed,
    Cancelled,
}

impl ShipmentState {
    /// Forward order of progress; a later rank wins. `Cancelled` is 0: it is
    /// handled separately (only valid before pickup).
    pub fn rank(self) -> u8 {
        match self {
            ShipmentState::Pending => 1,
            ShipmentState::Confirmed => 2,
            ShipmentState::PickedUp => 3,
            ShipmentState::InTransit => 4,
            ShipmentState::Completed => 5,
            ShipmentState::Cancelled => 0,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct CourierUpdate {
    pub provider_order_id: String,
    pub state: ShipmentState,
    pub occurred_at: Option<OffsetDateTime>,
    pub courier_phone: Option<String>,
    pub tracking_url: Option<String>,
}

impl fmt::Debug for CourierUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CourierUpdate")
            .field("provider_order_id", &self.provider_order_id)
            .field("state", &self.state)
            .field("occurred_at", &self.occurred_at)
            .finish_non_exhaustive()
    }
}

/// A courier's failure. `Display`/`Debug` print only the kind, never the inner
/// detail, which may echo provider responses.
#[non_exhaustive]
pub enum CourierError {
    /// The provider received and refused the request (also: insufficient wallet).
    Rejected {
        code: String,
        message: String,
    },
    /// The outcome is unknown: the request may have taken effect. Never re-book.
    Timeout,
    /// The call definitely did not take effect.
    Unavailable,
    /// The provider's answer cannot settle the question.
    Inconclusive,
    /// A webhook's signature did not match.
    InvalidSignature,
    NotSupported,
}

impl CourierError {
    fn kind(&self) -> &'static str {
        match self {
            CourierError::Rejected { .. } => "rejected",
            CourierError::Timeout => "timed out",
            CourierError::Unavailable => "unavailable",
            CourierError::Inconclusive => "inconclusive",
            CourierError::InvalidSignature => "invalid webhook signature",
            CourierError::NotSupported => "not supported",
        }
    }
}

impl fmt::Display for CourierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "courier provider: {}", self.kind())
    }
}

impl fmt::Debug for CourierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CourierError::Rejected { .. } => f.write_str("Rejected { .. }"),
            CourierError::Timeout => f.write_str("Timeout"),
            CourierError::Unavailable => f.write_str("Unavailable"),
            CourierError::Inconclusive => f.write_str("Inconclusive"),
            CourierError::InvalidSignature => f.write_str("InvalidSignature"),
            CourierError::NotSupported => f.write_str("NotSupported"),
        }
    }
}

impl std::error::Error for CourierError {}

impl From<CourierError> for AppError {
    fn from(e: CourierError) -> Self {
        tracing::warn!(kind = e.kind(), "courier provider error");
        match e {
            CourierError::NotSupported => AppError::NotSupported,
            CourierError::InvalidSignature => AppError::InvalidSignature,
            _ => AppError::ProviderUnavailable,
        }
    }
}

#[async_trait]
pub trait CourierProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// Validates an address and returns the provider's address code.
    async fn validate_address(&self, addr: &ContactAddress) -> Result<i64, CourierError>;

    async fn quote(
        &self,
        sender_code: i64,
        receiver_code: i64,
        pickup: Date,
        parcel: &Parcel,
    ) -> Result<RateQuote, CourierError>;

    /// Books a label. There is no idempotency key: never retry on `Timeout`.
    async fn book(
        &self,
        request_token: &str,
        courier_id: &str,
        service_code: &str,
    ) -> Result<Booking, CourierError>;

    async fn cancel(&self, provider_order_id: &str) -> Result<(), CourierError>;

    async fn status(&self, provider_order_id: &str) -> Result<CourierUpdate, CourierError>;

    /// Verifies and parses a webhook. `None` means an ignored event.
    fn parse_webhook(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<Option<CourierUpdate>, CourierError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(l: Option<i32>, w: Option<i32>, h: Option<i32>) -> ProductDims {
        ProductDims {
            name: "x".into(),
            weight_grams: 500,
            unit_amount_kobo: 10_000,
            length_cm: l,
            width_cm: w,
            height_cm: h,
        }
    }

    #[test]
    fn parcel_for_stacks_heights_and_defaults_dims() {
        let p = parcel_for(
            &[
                (dims(Some(30), Some(20), Some(5)), 3),
                (dims(None, Some(25), None), 2),
            ],
            7,
        );
        assert_eq!(p.length_cm, 30);
        assert_eq!(p.width_cm, 25);
        assert_eq!(p.height_cm, 5 * 3 + 10 * 2);
        assert_eq!(p.category_id, 7);
        assert_eq!(p.items.len(), 2);
        assert_eq!(p.items[0].quantity, 3);
        // Huge quantities cap instead of overflowing.
        let big = parcel_for(&[(dims(None, None, Some(50)), i32::MAX)], 1);
        assert_eq!(big.height_cm, 300);
    }

    #[test]
    fn shipment_state_rank_is_forward_order() {
        use ShipmentState::*;
        let ranks: Vec<u8> = [Pending, Confirmed, PickedUp, InTransit, Completed]
            .iter()
            .map(|s| s.rank())
            .collect();
        assert_eq!(ranks, vec![1, 2, 3, 4, 5]);
        assert_eq!(Cancelled.rank(), 0);
    }

    #[test]
    fn debug_redacts_personal_fields() {
        let a = ContactAddress {
            name: "Ada Obi".into(),
            email: Some("ada@example.com".into()),
            phone: "08012345678".into(),
            address_line: "1 Secret Street".into(),
            area: AreaCouncil::Amac,
        };
        let s = format!("{a:?}");
        assert!(!s.contains("Ada") && !s.contains("0801") && !s.contains("Secret"));
        let u = CourierUpdate {
            provider_order_id: "o1".into(),
            state: ShipmentState::PickedUp,
            occurred_at: None,
            courier_phone: Some("08099999999".into()),
            tracking_url: None,
        };
        assert!(!format!("{u:?}").contains("0809"));
    }
}
