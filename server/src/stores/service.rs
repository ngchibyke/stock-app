use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    audit,
    crypto::FieldCipher,
    delivery::area::{AreaCouncil, normalize_postcode},
    error::AppError,
    phone::normalize_ng_phone,
};

pub const MAX_STORES_PER_USER: i64 = 2;
pub const MAX_SELF_DELIVERY_FEE_KOBO: i64 = 5_000_000;
const SERVICE_STATE: &str = "FCT";
const MAX_SLUG_ATTEMPTS: u32 = 1000;
/// Slugs that collide with static routes (e.g. `/stores/mine`); never allocated as-is.
const RESERVED_SLUGS: &[&str] = &["mine", "new", "search", "id"];

#[derive(Clone, Deserialize)]
pub struct NewStore {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub pickup_address: String,
    pub pickup_state: String,
    pub delivery_courier: bool,
    pub delivery_self: bool,
    pub pickup_area_council: Option<String>,
    pub pickup_postcode: Option<String>,
    pub business_phone: Option<String>,
    pub self_delivery_fee_kobo: Option<i64>,
}

#[derive(Default, Deserialize)]
pub struct StorePatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub pickup_address: Option<String>,
    pub pickup_state: Option<String>,
    pub delivery_courier: Option<bool>,
    pub delivery_self: Option<bool>,
    pub pickup_area_council: Option<String>,
    /// Blank clears the postcode.
    pub pickup_postcode: Option<String>,
    pub business_phone: Option<String>,
    pub self_delivery_fee_kobo: Option<i64>,
}

const REDACTED: &str = "[redacted]";

// Hand-written so the pickup address (personal data) never reaches logs or panics.
impl std::fmt::Debug for NewStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewStore")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("pickup_address", &REDACTED)
            .field("pickup_state", &self.pickup_state)
            .field("delivery_courier", &self.delivery_courier)
            .field("delivery_self", &self.delivery_self)
            .field("pickup_area_council", &self.pickup_area_council)
            .field(
                "pickup_postcode",
                &self.pickup_postcode.as_ref().map(|_| REDACTED),
            )
            .field(
                "business_phone",
                &self.business_phone.as_ref().map(|_| REDACTED),
            )
            .field("self_delivery_fee_kobo", &self.self_delivery_fee_kobo)
            .finish()
    }
}

impl std::fmt::Debug for StorePatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorePatch")
            .field("name", &self.name)
            .field("description", &self.description)
            .field(
                "pickup_address",
                &self.pickup_address.as_ref().map(|_| REDACTED),
            )
            .field("pickup_state", &self.pickup_state)
            .field("delivery_courier", &self.delivery_courier)
            .field("delivery_self", &self.delivery_self)
            .field("pickup_area_council", &self.pickup_area_council)
            .field(
                "pickup_postcode",
                &self.pickup_postcode.as_ref().map(|_| REDACTED),
            )
            .field(
                "business_phone",
                &self.business_phone.as_ref().map(|_| REDACTED),
            )
            .field("self_delivery_fee_kobo", &self.self_delivery_fee_kobo)
            .finish()
    }
}

/// A store as stored, with the pickup address still encrypted.
#[derive(Debug, sqlx::FromRow)]
pub struct Store {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub pickup_address_enc: Vec<u8>,
    pub pickup_state: String,
    pub delivery_courier: bool,
    pub delivery_self: bool,
    pub status: String,
    pub created_at: OffsetDateTime,
    pub pickup_area_council: Option<AreaCouncil>,
    pub pickup_postcode: Option<String>,
    pub pickup_courier_address_code: Option<i64>,
    pub business_phone_enc: Option<Vec<u8>>,
    pub self_delivery_fee_kobo: Option<i64>,
}

const COLUMNS: &str = "id, owner_id, name, slug, description, pickup_address_enc, pickup_state, \
                       delivery_courier, delivery_self, status::text AS status, created_at, \
                       pickup_area_council, pickup_postcode, pickup_courier_address_code, \
                       business_phone_enc, self_delivery_fee_kobo";

/// The public shape: never carries the pickup address.
#[derive(Debug, Serialize)]
pub struct PublicStore {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub status: String,
    pub pickup_state: String,
    pub delivery_courier: bool,
    pub delivery_self: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// The owner-only shape: the public shape plus the decrypted pickup address and
/// the delivery details.
#[derive(Serialize)]
pub struct OwnerStore {
    #[serde(flatten)]
    pub store: PublicStore,
    pub pickup_address: String,
    pub pickup_area_council: Option<AreaCouncil>,
    pub pickup_postcode: Option<String>,
    pub business_phone: Option<String>,
    pub self_delivery_fee_kobo: Option<i64>,
}

impl std::fmt::Debug for OwnerStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerStore")
            .field("store", &self.store)
            .field("pickup_address", &REDACTED)
            .field("pickup_area_council", &self.pickup_area_council)
            .field("pickup_postcode", &REDACTED)
            .field("business_phone", &REDACTED)
            .field("self_delivery_fee_kobo", &self.self_delivery_fee_kobo)
            .finish()
    }
}

/// What checkout needs to know about how a store delivers (Task 6). Decrypted;
/// `Debug` redacts the personal fields.
#[derive(Clone)]
pub struct StoreDelivery {
    pub courier: bool,
    /// `delivery_self`: a fee left on the row after self-delivery is switched
    /// off must not be offered.
    pub self_delivery: bool,
    pub self_fee_kobo: Option<i64>,
    pub business_phone: Option<String>,
    pub pickup_area: Option<AreaCouncil>,
    pub pickup_address: String,
    pub pickup_courier_address_code: Option<i64>,
}

impl std::fmt::Debug for StoreDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreDelivery")
            .field("courier", &self.courier)
            .field("self_delivery", &self.self_delivery)
            .field("self_fee_kobo", &self.self_fee_kobo)
            .field(
                "business_phone",
                &self.business_phone.as_ref().map(|_| REDACTED),
            )
            .field("pickup_area", &self.pickup_area)
            .field("pickup_address", &REDACTED)
            .field(
                "pickup_courier_address_code",
                &self.pickup_courier_address_code,
            )
            .finish()
    }
}

impl Store {
    pub fn public(&self) -> PublicStore {
        PublicStore {
            id: self.id,
            name: self.name.clone(),
            slug: self.slug.clone(),
            description: self.description.clone(),
            status: self.status.clone(),
            pickup_state: self.pickup_state.clone(),
            delivery_courier: self.delivery_courier,
            delivery_self: self.delivery_self,
            created_at: self.created_at,
        }
    }

    pub fn owner_view(&self, cipher: &FieldCipher) -> Result<OwnerStore, AppError> {
        Ok(OwnerStore {
            store: self.public(),
            pickup_address: cipher.decrypt(&self.pickup_address_enc)?,
            pickup_area_council: self.pickup_area_council,
            pickup_postcode: self.pickup_postcode.clone(),
            business_phone: self
                .business_phone_enc
                .as_deref()
                .map(|b| cipher.decrypt(b))
                .transpose()?,
            self_delivery_fee_kobo: self.self_delivery_fee_kobo,
        })
    }
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "store".to_string()
    } else {
        out.to_string()
    }
}

fn validate_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(AppError::Validation(
            "name must be 1 to 80 characters".into(),
        ));
    }
    Ok(name.to_string())
}

fn validate_description(description: &str) -> Result<(), AppError> {
    if description.chars().count() > 2000 {
        return Err(AppError::Validation(
            "description must be at most 2000 characters".into(),
        ));
    }
    Ok(())
}

fn validate_address(address: &str) -> Result<String, AppError> {
    let address = address.trim();
    if address.is_empty() || address.chars().count() > 300 {
        return Err(AppError::Validation(
            "pickup_address must be 1 to 300 characters".into(),
        ));
    }
    Ok(address.to_string())
}

fn validate_state(state: &str) -> Result<(), AppError> {
    if state != SERVICE_STATE {
        return Err(AppError::Unprocessable(
            "outside_service_area",
            "Stores must currently be located in the FCT".into(),
        ));
    }
    Ok(())
}

fn validate_delivery(courier: bool, own: bool) -> Result<(), AppError> {
    if !courier && !own {
        return Err(AppError::Validation(
            "at least one delivery method is required".into(),
        ));
    }
    Ok(())
}

/// Checks the full delivery state: each method switched on needs its details.
fn validate_delivery_details(
    courier: bool,
    own: bool,
    fee: Option<i64>,
    has_phone: bool,
    area: Option<AreaCouncil>,
) -> Result<(), AppError> {
    if let Some(fee) = fee
        && !(0..=MAX_SELF_DELIVERY_FEE_KOBO).contains(&fee)
    {
        return Err(AppError::Validation(format!(
            "self_delivery_fee_kobo must be between 0 and {MAX_SELF_DELIVERY_FEE_KOBO}"
        )));
    }
    let required = |msg: &str| AppError::Unprocessable("delivery_details_required", msg.into());
    if own && (fee.is_none() || !has_phone) {
        return Err(required(match (fee.is_none(), has_phone) {
            (true, false) => "Self-delivery needs a fee and a business phone",
            (true, true) => "Self-delivery needs a fee",
            _ => "Self-delivery needs a business phone",
        }));
    }
    if courier && area.is_none() {
        return Err(required("Courier delivery needs a pickup area council"));
    }
    // The courier needs a sender phone to collect the parcel (ruling T6d).
    if courier && !has_phone {
        return Err(required("Courier delivery needs a business phone"));
    }
    Ok(())
}

/// Blank means absent.
fn parse_postcode(raw: &str) -> Result<Option<String>, AppError> {
    match raw.trim() {
        "" => Ok(None),
        p => Ok(Some(normalize_postcode(p)?)),
    }
}

pub async fn create_store(
    db: &PgPool,
    cipher: &FieldCipher,
    owner: Uuid,
    input: NewStore,
) -> Result<Store, AppError> {
    validate_state(&input.pickup_state)?;
    let name = validate_name(&input.name)?;
    validate_description(&input.description)?;
    let address = validate_address(&input.pickup_address)?;
    validate_delivery(input.delivery_courier, input.delivery_self)?;
    let area: Option<AreaCouncil> = input
        .pickup_area_council
        .as_deref()
        .filter(|a| !a.trim().is_empty())
        .map(str::parse)
        .transpose()?;
    let postcode = input
        .pickup_postcode
        .as_deref()
        .map(parse_postcode)
        .transpose()?
        .flatten();
    let phone = input
        .business_phone
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(normalize_ng_phone)
        .transpose()?;
    validate_delivery_details(
        input.delivery_courier,
        input.delivery_self,
        input.self_delivery_fee_kobo,
        phone.is_some(),
        area,
    )?;
    let address_enc = cipher.encrypt(&address);
    let phone_enc = phone.as_deref().map(|p| cipher.encrypt(p));

    let mut tx = db.begin().await.context("begin")?;
    // Serialise concurrent creates for the same user (lock order: users row first).
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(owner)
        .fetch_one(&mut *tx)
        .await
        .context("lock user")?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM stores WHERE owner_id = $1 AND deleted_at IS NULL",
    )
    .bind(owner)
    .fetch_one(&mut *tx)
    .await
    .context("count stores")?;
    if count >= MAX_STORES_PER_USER {
        return Err(AppError::Conflict("store_limit_reached"));
    }

    let base = slugify(&name);
    let mut store = None;
    for n in 1..=MAX_SLUG_ATTEMPTS {
        if n == 1 && RESERVED_SLUGS.contains(&base.as_str()) {
            continue;
        }
        let slug = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        // The unique index decides; a taken slug (even a deleted store's) yields no row.
        let inserted: Option<Store> = sqlx::query_as(&format!(
            "INSERT INTO stores (owner_id, name, slug, description, pickup_address_enc, pickup_state, \
                                 delivery_courier, delivery_self, pickup_area_council, pickup_postcode, \
                                 business_phone_enc, self_delivery_fee_kobo) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
             ON CONFLICT (slug) DO NOTHING RETURNING {COLUMNS}"
        ))
        .bind(owner)
        .bind(&name)
        .bind(&slug)
        .bind(&input.description)
        .bind(&address_enc)
        .bind(&input.pickup_state)
        .bind(input.delivery_courier)
        .bind(input.delivery_self)
        .bind(area)
        .bind(&postcode)
        .bind(&phone_enc)
        .bind(input.self_delivery_fee_kobo)
        .fetch_optional(&mut *tx)
        .await
        .context("insert store")?;
        if inserted.is_some() {
            store = inserted;
            break;
        }
    }
    let store = store.ok_or_else(|| anyhow::anyhow!("could not allocate a store slug"))?;
    audit::record(
        &mut tx,
        Some(owner),
        "store.created",
        "store",
        Some(store.id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(store)
}

pub async fn list_mine(db: &PgPool, owner: Uuid) -> Result<Vec<Store>, AppError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM stores WHERE owner_id = $1 AND deleted_at IS NULL ORDER BY created_at, id"
    ))
    .bind(owner)
    .fetch_all(db)
    .await
    .context("list stores")?)
}

/// Active stores are public; the owner can also see their own store in any status.
pub async fn get_by_slug(db: &PgPool, slug: &str, viewer: Option<Uuid>) -> Result<Store, AppError> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM stores \
         WHERE slug = $1 AND deleted_at IS NULL AND (status = 'active' OR owner_id = $2)"
    ))
    .bind(slug)
    .bind(viewer)
    .fetch_optional(db)
    .await
    .context("get store")?
    .ok_or(AppError::NotFound)
}

pub async fn update_store(
    db: &PgPool,
    cipher: &FieldCipher,
    owner: Uuid,
    id: Uuid,
    patch: StorePatch,
) -> Result<Store, AppError> {
    let mut tx = db.begin().await.context("begin")?;
    let current = lock_owned(&mut tx, owner, id).await?;

    if let Some(state) = &patch.pickup_state {
        validate_state(state)?;
    }
    let name = match &patch.name {
        Some(n) => validate_name(n)?,
        None => current.name.clone(),
    };
    let description = patch
        .description
        .clone()
        .unwrap_or_else(|| current.description.clone());
    validate_description(&description)?;
    let address_enc = match &patch.pickup_address {
        Some(a) => cipher.encrypt(&validate_address(a)?),
        None => current.pickup_address_enc.clone(),
    };
    let state = patch
        .pickup_state
        .clone()
        .unwrap_or_else(|| current.pickup_state.clone());
    let courier = patch.delivery_courier.unwrap_or(current.delivery_courier);
    let own = patch.delivery_self.unwrap_or(current.delivery_self);
    validate_delivery(courier, own)?;

    let area = match patch
        .pickup_area_council
        .as_deref()
        .filter(|a| !a.trim().is_empty())
    {
        Some(a) => Some(a.parse::<AreaCouncil>()?),
        None => current.pickup_area_council,
    };
    let postcode = match patch.pickup_postcode.as_deref() {
        Some(p) => parse_postcode(p)?,
        None => current.pickup_postcode.clone(),
    };
    let new_phone = patch
        .business_phone
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(normalize_ng_phone)
        .transpose()?;
    let fee = patch
        .self_delivery_fee_kobo
        .or(current.self_delivery_fee_kobo);
    let phone_enc = match &new_phone {
        Some(p) => Some(cipher.encrypt(p)),
        None => current.business_phone_enc.clone(),
    };

    let mut changed: Vec<&str> = Vec::new();
    if courier != current.delivery_courier {
        changed.push("delivery_courier");
    }
    if own != current.delivery_self {
        changed.push("delivery_self");
    }
    if area != current.pickup_area_council {
        changed.push("pickup_area_council");
    }
    if postcode != current.pickup_postcode {
        changed.push("pickup_postcode");
    }
    if fee != current.self_delivery_fee_kobo {
        changed.push("self_delivery_fee_kobo");
    }
    if let Some(p) = &new_phone {
        let same = current
            .business_phone_enc
            .as_deref()
            .map(|b| cipher.decrypt(b))
            .transpose()?
            .is_some_and(|old| &old == p);
        if !same {
            changed.push("business_phone");
        }
    }
    let touched = patch.delivery_courier.is_some()
        || patch.delivery_self.is_some()
        || patch.pickup_area_council.is_some()
        || patch.pickup_postcode.is_some()
        || patch.business_phone.is_some()
        || patch.self_delivery_fee_kobo.is_some();
    // A legacy store with NULL delivery fields stays editable until a delivery field is touched.
    if touched {
        validate_delivery_details(courier, own, fee, phone_enc.is_some(), area)?;
    }
    // A moved pickup, or a new sender phone, invalidates the courier's saved
    // address code (checkout's delivery options refill it).
    let pickup_moved = patch.pickup_address.is_some()
        || changed.contains(&"pickup_area_council")
        || changed.contains(&"pickup_postcode")
        || changed.contains(&"business_phone");
    let courier_code = if pickup_moved {
        None
    } else {
        current.pickup_courier_address_code
    };

    let store: Store = sqlx::query_as(&format!(
        "UPDATE stores SET name = $2, description = $3, pickup_address_enc = $4, pickup_state = $5, \
                           delivery_courier = $6, delivery_self = $7, pickup_area_council = $8, \
                           pickup_postcode = $9, business_phone_enc = $10, \
                           self_delivery_fee_kobo = $11, pickup_courier_address_code = $12 \
         WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(&name)
    .bind(&description)
    .bind(&address_enc)
    .bind(&state)
    .bind(courier)
    .bind(own)
    .bind(area)
    .bind(&postcode)
    .bind(&phone_enc)
    .bind(fee)
    .bind(courier_code)
    .fetch_one(&mut *tx)
    .await
    .context("update store")?;
    // Field names only, never values.
    let detail = if changed.is_empty() {
        json!({})
    } else {
        json!({ "delivery_fields_changed": changed })
    };
    audit::record(
        &mut tx,
        Some(owner),
        "store.updated",
        "store",
        Some(id),
        detail,
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(store)
}

/// How a live store delivers, decrypted, for checkout.
pub async fn delivery_profile(
    db: &PgPool,
    cipher: &FieldCipher,
    store_id: Uuid,
) -> Result<StoreDelivery, AppError> {
    let store: Store = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM stores WHERE id = $1 AND deleted_at IS NULL"
    ))
    .bind(store_id)
    .fetch_optional(db)
    .await
    .context("load store delivery")?
    .ok_or(AppError::NotFound)?;
    Ok(StoreDelivery {
        courier: store.delivery_courier,
        self_delivery: store.delivery_self,
        self_fee_kobo: store.self_delivery_fee_kobo,
        business_phone: store
            .business_phone_enc
            .as_deref()
            .map(|b| cipher.decrypt(b))
            .transpose()?,
        pickup_area: store.pickup_area_council,
        pickup_address: cipher.decrypt(&store.pickup_address_enc)?,
        pickup_courier_address_code: store.pickup_courier_address_code,
    })
}

pub async fn delete_store(db: &PgPool, owner: Uuid, id: Uuid) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    lock_owned(&mut tx, owner, id).await?;
    sqlx::query("UPDATE stores SET deleted_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .context("soft delete store")?;
    audit::record(
        &mut tx,
        Some(owner),
        "store.deleted",
        "store",
        Some(id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(())
}

/// Locks a live store row owned by `owner`; anything else is NotFound (existence is not revealed).
async fn lock_owned(
    tx: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    id: Uuid,
) -> Result<Store, AppError> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM stores WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL FOR NO KEY UPDATE"
    ))
    .bind(id)
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await
    .context("lock store")?
    .ok_or(AppError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugs_are_hyphenated_lowercase() {
        assert_eq!(slugify("Mama  Put!"), "mama-put");
        assert_eq!(slugify("  --Abuja's Best--  "), "abuja-s-best");
        assert_eq!(slugify("!!!"), "store");
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn store_inputs_do_not_debug_print_the_address() {
        let new = NewStore {
            name: "Shop".into(),
            description: String::new(),
            pickup_address: "12 Secret Close, Wuse".into(),
            pickup_state: "FCT".into(),
            delivery_courier: true,
            delivery_self: false,
            pickup_area_council: Some("amac".into()),
            pickup_postcode: Some("FC01SECRET99".into()),
            business_phone: Some("08031234567".into()),
            self_delivery_fee_kobo: Some(100),
        };
        let patch = StorePatch {
            pickup_address: Some("12 Secret Close, Wuse".into()),
            business_phone: Some("08031234567".into()),
            pickup_postcode: Some("FC01SECRET99".into()),
            ..Default::default()
        };
        for printed in [format!("{new:?}"), format!("{patch:?}")] {
            assert!(!printed.contains("Secret"), "{printed}");
            assert!(!printed.contains("SECRET"), "{printed}");
            assert!(!printed.contains("0803"), "{printed}");
            assert!(printed.contains("[redacted]"), "{printed}");
        }
        // Fields without personal data stay visible for debugging.
        assert!(format!("{new:?}").contains("Shop"));
        // An absent address is shown as absent.
        assert!(format!("{:?}", StorePatch::default()).contains("pickup_address: None"));
    }
}
