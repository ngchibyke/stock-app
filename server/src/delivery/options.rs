//! Delivery options at checkout (Part 3, Task 6).
//!
//! `POST /checkout/delivery-options` prices delivery for every store in the
//! buyer's cart to one of their saved addresses and stores each choice as a
//! `delivery_options` row. The client only ever sends an option's id back to
//! `POST /checkout`; every amount comes from these rows, never from the client.
//!
//! - Self-delivery is offered when the store has it switched on with a fee.
//! - Courier delivery is offered when the store has it switched on, a business
//!   phone and a pickup area, and a courier provider is configured. The address
//!   codes of the store and of the buyer's address are fetched lazily and cached
//!   on their rows. At most the three cheapest rates (by what the courier
//!   charges) are offered.
//! - Every provider call happens before any transaction is opened; the caches and
//!   options are written afterwards. A provider failure leaves that store with
//!   self-delivery only; a store left with no option says `no_delivery_option`.
//!   Courier errors are logged by class only (their `Display` carries no detail).
//!
//! An option is valid at checkout only for the same buyer, store and address,
//! before it expires (`courier_quote_ttl_minutes`), and while the store's cart
//! lines are unchanged (`cart_fingerprint`); otherwise 422 `delivery_option_invalid`.
//!
//! Data protection: option rows hold ids and prices only. The buyer's name, phone
//! and address line go to the courier provider for address validation (needed for
//! the label); the landmark is not sent. Nothing personal is logged or audited.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Context;
use axum::{Json, Router, extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::{
    addresses::{DeliveryAddress, load_for_checkout},
    area::AreaCouncil,
    promise::{load_holidays, next_pickup_date, promised_date},
};
use crate::{
    AppState, audit,
    auth::AuthUser,
    cart::service::BUYABLE,
    couriers::{ContactAddress, CourierProvider, ProductDims, Rate, parcel_for},
    crypto::FieldCipher,
    error::AppError,
    extract::AppJson,
    products::service::unit_price,
    settings,
    stores::service::{StoreDelivery, delivery_profile},
};

/// Most courier rates offered per store (ruling T6f).
pub const MAX_COURIER_OPTIONS: usize = 3;
/// What a store with no deliverable option reports.
pub const NO_DELIVERY_OPTION: &str = "no_delivery_option";

/// How an order is delivered. Postgres `delivery_method`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "delivery_method")]
pub enum DeliveryMethod {
    #[serde(rename = "courier")]
    #[sqlx(rename = "courier")]
    Courier,
    /// The seller delivers.
    #[serde(rename = "self")]
    #[sqlx(rename = "self")]
    SelfDelivery,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/checkout/delivery-options", post(quote_options))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionsBody {
    address_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct OptionView {
    pub id: Uuid,
    pub method: DeliveryMethod,
    pub fee_kobo: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub courier_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_eta: Option<String>,
    /// `YYYY-MM-DD`, Africa/Lagos: the date promised if paid now.
    pub promised_date: String,
}

#[derive(Debug, Serialize)]
pub struct StoreOptions {
    pub store_id: Uuid,
    pub store_name: String,
    pub options: Vec<OptionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct OptionsResponse {
    pub stores: Vec<StoreOptions>,
}

/// sha256 hex over a store's cart lines `(variant_id, quantity)`, sorted, so the
/// order lines arrive in does not matter (ruling T6a).
pub fn cart_fingerprint(lines: &[(Uuid, i32)]) -> String {
    let mut sorted = lines.to_vec();
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    for (variant_id, quantity) in sorted {
        hasher.update(variant_id.as_bytes());
        hasher.update(quantity.to_be_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The buyer's own live address, decrypted, if it is inside the service area.
/// Another user's or a deleted address is `NotFound`; an area the service does
/// not cover (only reachable past the API) is 422 `outside_service_area`.
pub async fn checkout_address(
    db: &PgPool,
    cipher: &FieldCipher,
    buyer: Uuid,
    address_id: Uuid,
) -> Result<DeliveryAddress, AppError> {
    // Read as text, so a value outside the six councils is refused, never decoded.
    let area: Option<String> = sqlx::query_scalar(
        "SELECT area_council::text FROM buyer_addresses \
         WHERE id = $1 AND user_id = $2 AND deleted_at IS NULL",
    )
    .bind(address_id)
    .bind(buyer)
    .fetch_optional(db)
    .await
    .context("load address area")?;
    area.ok_or(AppError::NotFound)?.parse::<AreaCouncil>()?;
    load_for_checkout(db, cipher, buyer, address_id).await
}

#[derive(sqlx::FromRow)]
struct CartRow {
    store_id: Uuid,
    store_name: String,
    variant_id: Uuid,
    quantity: i32,
    product_name: String,
    base_price_kobo: i64,
    price_kobo: Option<i64>,
    weight_grams: i32,
    length_cm: Option<i32>,
    width_cm: Option<i32>,
    height_cm: Option<i32>,
}

struct StoreCart {
    name: String,
    lines: Vec<(Uuid, i32)>,
    items: Vec<(ProductDims, i32)>,
}

/// The buyer's buyable cart lines grouped by store (their own stores left out:
/// checkout refuses those items). An empty cart is 422 `cart_empty`.
async fn load_cart(db: &PgPool, buyer: Uuid) -> Result<Vec<(Uuid, StoreCart)>, AppError> {
    let any: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cart_items WHERE user_id = $1)")
            .bind(buyer)
            .fetch_one(db)
            .await
            .context("check cart")?;
    if !any {
        return Err(AppError::Unprocessable(
            "cart_empty",
            "Your cart is empty".into(),
        ));
    }
    let rows: Vec<CartRow> = sqlx::query_as(&format!(
        "SELECT s.id AS store_id, s.name AS store_name, c.variant_id, c.quantity, \
                p.name AS product_name, p.base_price_kobo, v.price_kobo, p.weight_grams, \
                p.length_cm, p.width_cm, p.height_cm \
         FROM cart_items c \
         JOIN product_variants v ON v.id = c.variant_id \
         JOIN products p ON p.id = v.product_id \
         JOIN stores s ON s.id = p.store_id \
         WHERE c.user_id = $1 AND s.owner_id <> $1 AND {BUYABLE} \
         ORDER BY s.name, s.id, c.variant_id"
    ))
    .bind(buyer)
    .fetch_all(db)
    .await
    .context("load cart for delivery")?;
    let mut out: Vec<(Uuid, StoreCart)> = Vec::new();
    for r in rows {
        if out.last().is_none_or(|(id, _)| *id != r.store_id) {
            out.push((
                r.store_id,
                StoreCart {
                    name: r.store_name.clone(),
                    lines: Vec::new(),
                    items: Vec::new(),
                },
            ));
        }
        let (_, cart) = out.last_mut().expect("just pushed");
        cart.lines.push((r.variant_id, r.quantity));
        // Ruling T6g: the parcel item is valued at the unit price, named after the product.
        cart.items.push((
            ProductDims {
                name: r.product_name,
                weight_grams: r.weight_grams,
                unit_amount_kobo: unit_price(r.base_price_kobo, r.price_kobo),
                length_cm: r.length_cm,
                width_cm: r.width_cm,
                height_cm: r.height_cm,
            },
            r.quantity,
        ));
    }
    Ok(out)
}

/// The buyer's address code with the courier provider, fetched at most once per request.
enum ReceiverCode {
    Unknown,
    Known(i64),
    Failed,
}

struct CourierQuote {
    request_token: String,
    rates: Vec<Rate>,
    /// A store address code fetched now, to cache after the provider calls.
    new_store_code: Option<(i64, ContactAddress)>,
}

/// Every provider call for one store: the store's address code (unless cached),
/// the buyer's (unless known), then the quote. `None` on any provider failure,
/// logged by error class only.
#[allow(clippy::too_many_arguments)]
async fn quote_courier(
    provider: &dyn CourierProvider,
    store_name: &str,
    profile: &StoreDelivery,
    address: &DeliveryAddress,
    receiver: &mut ReceiverCode,
    items: &[(ProductDims, i32)],
    category_id: i64,
    pickup: Date,
) -> Option<CourierQuote> {
    let (Some(phone), Some(area)) = (&profile.business_phone, profile.pickup_area) else {
        return None;
    };
    let mut new_store_code = None;
    let sender = match profile.pickup_courier_address_code {
        Some(code) => code,
        None => {
            // Shipbubble requires an email; the adapter sends the platform's when None.
            let contact = ContactAddress {
                name: store_name.to_string(),
                email: None,
                phone: phone.clone(),
                address_line: profile.pickup_address.clone(),
                area,
            };
            match provider.validate_address(&contact).await {
                Ok(code) => {
                    new_store_code = Some((code, contact));
                    code
                }
                Err(e) => {
                    tracing::warn!(error = %e, "store address validation failed; no courier option");
                    return None;
                }
            }
        }
    };
    let receiver_code = match receiver {
        ReceiverCode::Known(code) => *code,
        ReceiverCode::Failed => return None,
        ReceiverCode::Unknown => {
            let contact = ContactAddress {
                name: address.recipient_name.clone(),
                email: None,
                phone: address.phone.clone(),
                address_line: address.address_line.clone(),
                area: address.area_council,
            };
            match provider.validate_address(&contact).await {
                Ok(code) => {
                    *receiver = ReceiverCode::Known(code);
                    code
                }
                Err(e) => {
                    *receiver = ReceiverCode::Failed;
                    tracing::warn!(error = %e, "buyer address validation failed; no courier option");
                    return None;
                }
            }
        }
    };
    let parcel = parcel_for(items, category_id);
    match provider.quote(sender, receiver_code, pickup, &parcel).await {
        Ok(quote) => {
            let mut rates = quote.rates;
            rates.sort_by(|a, b| {
                (a.charged_kobo, &a.courier_name, &a.service_code).cmp(&(
                    b.charged_kobo,
                    &b.courier_name,
                    &b.service_code,
                ))
            });
            rates.truncate(MAX_COURIER_OPTIONS);
            Some(CourierQuote {
                request_token: quote.request_token,
                rates,
                new_store_code,
            })
        }
        Err(e) => {
            tracing::warn!(error = %e, "courier quote failed; no courier option");
            // The store's code is still good; cache it.
            new_store_code.map(|store_code| CourierQuote {
                request_token: String::new(),
                rates: Vec::new(),
                new_store_code: Some(store_code),
            })
        }
    }
}

/// Caches a store's courier address code, only while the pickup details it was
/// validated with are still the store's (a seller edit meanwhile resets it).
async fn cache_store_code(
    state: &AppState,
    store_id: Uuid,
    code: i64,
    sent: &ContactAddress,
) -> Result<(), AppError> {
    type PickupRow = (Vec<u8>, Option<Vec<u8>>, Option<AreaCouncil>, Option<i64>);
    let mut tx = state.db.begin().await.context("begin")?;
    let row: Option<PickupRow> = sqlx::query_as(
        "SELECT pickup_address_enc, business_phone_enc, pickup_area_council, \
                pickup_courier_address_code \
         FROM stores WHERE id = $1 AND deleted_at IS NULL FOR NO KEY UPDATE",
    )
    .bind(store_id)
    .fetch_optional(&mut *tx)
    .await
    .context("lock store for courier code")?;
    if let Some((address_enc, Some(phone_enc), area, None)) = row {
        let same = area == Some(sent.area)
            && state.cipher.decrypt(&address_enc)? == sent.address_line
            && state.cipher.decrypt(&phone_enc)? == sent.phone;
        if same {
            sqlx::query("UPDATE stores SET pickup_courier_address_code = $2 WHERE id = $1")
                .bind(store_id)
                .bind(code)
                .execute(&mut *tx)
                .await
                .context("cache store courier code")?;
        }
    }
    tx.commit().await.context("commit")?;
    Ok(())
}

struct NewOption {
    method: DeliveryMethod,
    fee_kobo: i64,
    rate: Option<(String, Rate)>,
}

async fn quote_options(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<OptionsBody>,
) -> Result<Json<OptionsResponse>, AppError> {
    Ok(Json(options_for(&state, user.id, body.address_id).await?))
}

/// Quotes and stores the delivery options for the buyer's cart (module docs).
pub async fn options_for(
    state: &AppState,
    buyer: Uuid,
    address_id: Uuid,
) -> Result<OptionsResponse, AppError> {
    let address = checkout_address(&state.db, &state.cipher, buyer, address_id).await?;
    let settings = settings::load(&state.db).await?;
    let holidays = load_holidays(&state.db).await?;
    let carts = load_cart(&state.db, buyer).await?;
    let now = OffsetDateTime::now_utc();
    let promised = promised_date(now, settings.delivery_cutoff_minutes, &holidays);
    let pickup = next_pickup_date(now, settings.delivery_cutoff_minutes, &holidays);

    // Provider calls first, with no transaction open.
    let mut receiver = match address.courier_address_code {
        Some(code) => ReceiverCode::Known(code),
        None => ReceiverCode::Unknown,
    };
    let mut planned: Vec<(Uuid, StoreCart, Vec<NewOption>)> = Vec::with_capacity(carts.len());
    let mut store_codes: Vec<(Uuid, i64, ContactAddress)> = Vec::new();
    for (store_id, cart) in carts {
        let profile = delivery_profile(&state.db, &state.cipher, store_id).await?;
        let mut options = Vec::new();
        if profile.self_delivery
            && let Some(fee) = profile.self_fee_kobo
        {
            options.push(NewOption {
                method: DeliveryMethod::SelfDelivery,
                fee_kobo: fee,
                rate: None,
            });
        }
        if profile.courier
            && let Some(provider) = &state.courier
            && let Some(quote) = quote_courier(
                provider.as_ref(),
                &cart.name,
                &profile,
                &address,
                &mut receiver,
                &cart.items,
                settings.courier_category_id,
                pickup,
            )
            .await
        {
            if let Some((code, sent)) = quote.new_store_code {
                store_codes.push((store_id, code, sent));
            }
            for rate in quote.rates {
                options.push(NewOption {
                    method: DeliveryMethod::Courier,
                    // Ruling T6f: the buyer pays what the courier will charge.
                    fee_kobo: rate.charged_kobo,
                    rate: Some((quote.request_token.clone(), rate)),
                });
            }
        }
        planned.push((store_id, cart, options));
    }

    // The caches, after every provider call.
    if address.courier_address_code.is_none()
        && let ReceiverCode::Known(code) = receiver
    {
        sqlx::query(
            "UPDATE buyer_addresses SET courier_address_code = $3 \
             WHERE id = $1 AND user_id = $2 AND courier_address_code IS NULL",
        )
        .bind(address.id)
        .bind(buyer)
        .bind(code)
        .execute(&state.db)
        .await
        .context("cache buyer courier code")?;
    }
    for (store_id, code, sent) in &store_codes {
        cache_store_code(state, *store_id, *code, sent).await?;
    }

    let mut tx = state.db.begin().await.context("begin")?;
    // This buyer's long-expired options that no order points at are dropped.
    sqlx::query(
        "DELETE FROM delivery_options d \
         WHERE d.buyer_id = $1 AND d.expires_at < now() - interval '1 day' \
           AND NOT EXISTS (SELECT 1 FROM orders o WHERE o.delivery_option_id = d.id)",
    )
    .bind(buyer)
    .execute(&mut *tx)
    .await
    .context("prune delivery options")?;
    let mut stores = Vec::with_capacity(planned.len());
    let mut offered = 0usize;
    for (store_id, cart, options) in planned {
        let fingerprint = cart_fingerprint(&cart.lines);
        let mut views = Vec::with_capacity(options.len());
        for o in options {
            let (token, rate) = match &o.rate {
                Some((token, rate)) => (Some(token.as_str()), Some(rate)),
                None => (None, None),
            };
            let id: Uuid = sqlx::query_scalar(
                "INSERT INTO delivery_options \
                 (buyer_id, store_id, address_id, method, fee_kobo, cart_fingerprint, \
                  courier_id, service_code, request_token, courier_name, delivery_eta, \
                  charged_kobo, promised_date, expires_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                         now() + make_interval(mins => $14)) \
                 RETURNING id",
            )
            .bind(buyer)
            .bind(store_id)
            .bind(address.id)
            .bind(o.method)
            .bind(o.fee_kobo)
            .bind(&fingerprint)
            .bind(rate.map(|r| r.courier_id.as_str()))
            .bind(rate.map(|r| r.service_code.as_str()))
            .bind(token)
            .bind(rate.map(|r| r.courier_name.as_str()))
            .bind(rate.and_then(|r| r.delivery_eta.as_deref()))
            .bind(rate.map(|r| r.charged_kobo))
            .bind(promised)
            .bind(settings.courier_quote_ttl_minutes)
            .fetch_one(&mut *tx)
            .await
            .context("insert delivery option")?;
            views.push(OptionView {
                id,
                method: o.method,
                fee_kobo: o.fee_kobo,
                courier_name: rate.map(|r| r.courier_name.clone()),
                delivery_eta: rate.and_then(|r| r.delivery_eta.clone()),
                promised_date: promised.to_string(),
            });
        }
        offered += views.len();
        stores.push(StoreOptions {
            store_id,
            store_name: cart.name,
            unavailable: views.is_empty().then_some(NO_DELIVERY_OPTION),
            options: views,
        });
    }
    let unavailable = stores.iter().filter(|s| s.unavailable.is_some()).count();
    audit::record(
        &mut tx,
        Some(buyer),
        "delivery.options_quoted",
        "buyer_address",
        Some(address.id),
        json!({ "stores": stores.len(), "options": offered, "stores_unavailable": unavailable }),
    )
    .await?;
    tx.commit().await.context("commit delivery options")?;
    Ok(OptionsResponse { stores })
}

/// One `{store_id, option_id}` of the checkout body.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryChoice {
    pub store_id: Uuid,
    pub option_id: Uuid,
}

/// A stored option, validated for one order of a checkout.
#[derive(Debug, Clone)]
pub struct ChosenOption {
    pub id: Uuid,
    pub method: DeliveryMethod,
    pub fee_kobo: i64,
    pub courier_id: Option<String>,
    pub service_code: Option<String>,
    pub request_token: Option<String>,
    pub charged_kobo: Option<i64>,
    pub promised_date: Date,
}

#[derive(sqlx::FromRow)]
struct OptionRow {
    id: Uuid,
    buyer_id: Uuid,
    store_id: Uuid,
    address_id: Uuid,
    method: DeliveryMethod,
    fee_kobo: i64,
    cart_fingerprint: String,
    courier_id: Option<String>,
    service_code: Option<String>,
    request_token: Option<String>,
    charged_kobo: Option<i64>,
    promised_date: Date,
    live: bool,
}

fn option_invalid() -> AppError {
    AppError::Unprocessable(
        "delivery_option_invalid",
        "A delivery option is no longer valid; please choose delivery again".into(),
    )
}

/// Validates the checkout's choices against the cart (`store -> its lines`):
/// exactly one live option per store, the buyer's own, quoted for this address
/// and for these exact lines. Unknown, foreign, expired, mismatched, duplicate or
/// extra choices are 422 `delivery_option_invalid`; a store without a choice is
/// 422 `delivery_option_required`.
pub async fn chosen_options(
    conn: &mut PgConnection,
    buyer: Uuid,
    address_id: Uuid,
    choices: &[DeliveryChoice],
    carts: &BTreeMap<Uuid, Vec<(Uuid, i32)>>,
) -> Result<HashMap<Uuid, ChosenOption>, AppError> {
    let mut seen = HashSet::new();
    for c in choices {
        if !carts.contains_key(&c.store_id) || !seen.insert(c.store_id) {
            return Err(option_invalid());
        }
    }
    let ids: Vec<Uuid> = choices.iter().map(|c| c.option_id).collect();
    let mut rows: HashMap<Uuid, OptionRow> = sqlx::query_as::<_, OptionRow>(
        "SELECT id, buyer_id, store_id, address_id, method, fee_kobo, cart_fingerprint, \
                courier_id, service_code, request_token, charged_kobo, promised_date, \
                expires_at > now() AS live \
         FROM delivery_options WHERE id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await
    .context("load delivery options")?
    .into_iter()
    .map(|r| (r.id, r))
    .collect();
    let mut chosen = HashMap::with_capacity(choices.len());
    for c in choices {
        let row = rows.remove(&c.option_id).ok_or_else(option_invalid)?;
        let lines = &carts[&c.store_id];
        let valid = row.buyer_id == buyer
            && row.store_id == c.store_id
            && row.address_id == address_id
            && row.live
            && row.cart_fingerprint == cart_fingerprint(lines);
        if !valid {
            return Err(option_invalid());
        }
        chosen.insert(
            c.store_id,
            ChosenOption {
                id: row.id,
                method: row.method,
                fee_kobo: row.fee_kobo,
                courier_id: row.courier_id,
                service_code: row.service_code,
                request_token: row.request_token,
                charged_kobo: row.charged_kobo,
                promised_date: row.promised_date,
            },
        );
    }
    if carts.keys().any(|store| !chosen.contains_key(store)) {
        return Err(AppError::Unprocessable(
            "delivery_option_required",
            "Choose a delivery option for every store".into(),
        ));
    }
    Ok(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_ignores_line_order_but_not_quantities() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let f = cart_fingerprint(&[(a, 1), (b, 2)]);
        assert_eq!(f.len(), 64);
        assert!(
            f.bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_eq!(f, cart_fingerprint(&[(b, 2), (a, 1)]));
        assert_ne!(f, cart_fingerprint(&[(a, 1), (b, 3)]));
        assert_ne!(f, cart_fingerprint(&[(a, 1)]));
    }

    #[test]
    fn delivery_method_serialises_as_courier_or_self() {
        assert_eq!(
            serde_json::to_value(DeliveryMethod::SelfDelivery).unwrap(),
            "self"
        );
        assert_eq!(
            serde_json::to_value(DeliveryMethod::Courier).unwrap(),
            "courier"
        );
    }
}
