use std::collections::BTreeMap;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    audit,
    error::AppError,
    limits::MAX_PRICE_KOBO,
    stock::{self, BranchCount, MAX_AVAILABLE_SQL, StockInput},
    stores::access::{self, Action, Membership, is_member_sql},
    uploads::{
        claim::{self, Kind, claim},
        service::Purpose,
        store::ObjectStore,
    },
};

const MAX_VARIANTS: usize = 50;
const MAX_ATTRIBUTES: usize = 5;
const CONDITIONS: &[&str] = &["new", "used", "refurbished"];

#[derive(Debug, Clone, Deserialize)]
pub struct VariantInput {
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    /// A number for a store with one open branch, else `{branch_id: count}`.
    pub stock: StockInput,
    pub price_kobo: Option<i64>,
    pub sku: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NewProduct {
    pub name: String,
    pub description: String,
    pub category: String,
    pub brand: Option<String>,
    pub condition: String,
    pub photo_keys: Vec<String>,
    pub weight_grams: i32,
    pub length_cm: Option<i32>,
    pub width_cm: Option<i32>,
    pub height_cm: Option<i32>,
    pub base_price_kobo: i64,
    pub variants: Vec<VariantInput>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ProductPatch {
    pub name: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub brand: Option<String>,
    pub condition: Option<String>,
    pub photo_keys: Option<Vec<String>>,
    pub weight_grams: Option<i32>,
    pub length_cm: Option<i32>,
    pub width_cm: Option<i32>,
    pub height_cm: Option<i32>,
    pub base_price_kobo: Option<i64>,
    pub is_published: Option<bool>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Product {
    pub id: Uuid,
    pub store_id: Uuid,
    pub name: String,
    pub description: String,
    pub category: String,
    pub brand: Option<String>,
    pub condition: String,
    pub photo_keys: Vec<String>,
    pub weight_grams: i32,
    pub length_cm: Option<i32>,
    pub width_cm: Option<i32>,
    pub height_cm: Option<i32>,
    pub base_price_kobo: i64,
    pub is_published: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

const COLUMNS: &str = "id, store_id, name, description, category, brand, condition::text AS condition, \
                       photo_keys, weight_grams, length_cm, width_cm, height_cm, base_price_kobo, \
                       is_published, created_at, updated_at";

/// `COLUMNS` qualified for queries that join `stores` (which also has `id`, `name`, ...).
const PRODUCT_COLUMNS: &str = "p.id, p.store_id, p.name, p.description, p.category, p.brand, \
                       p.condition::text AS condition, p.photo_keys, p.weight_grams, p.length_cm, \
                       p.width_cm, p.height_cm, p.base_price_kobo, p.is_published, p.created_at, p.updated_at";

#[derive(Debug, Clone, Serialize)]
pub struct Variant {
    pub id: Uuid,
    pub attributes: BTreeMap<String, String>,
    /// Units a buyer can still buy in one line: the most one open branch holds
    /// beyond what pending checkouts reserve there.
    pub available_stock: i32,
    /// On-hand stock over the open branches. Shown to the store's team only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stock: Option<i32>,
    /// Units held by pending checkouts, over the open branches. Team only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved_quantity: Option<i32>,
    /// Each branch's counts. Team only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branches: Option<Vec<BranchCount>>,
    pub price_kobo: Option<i64>,
    pub sku: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct VariantRow {
    id: Uuid,
    attributes: serde_json::Value,
    stock: i32,
    reserved_quantity: i32,
    available: i64,
    price_kobo: Option<i64>,
    sku: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProductView {
    #[serde(flatten)]
    pub product: Product,
    /// Public addresses of `photo_keys`, in the same order. Filled by
    /// [`ProductView::with_urls`]; empty until then.
    pub photo_urls: Vec<String>,
    pub variants: Vec<Variant>,
}

impl ProductView {
    pub fn with_urls(mut self, objects: &dyn ObjectStore) -> Self {
        self.photo_urls = claim::public_urls(objects, &self.product.photo_keys);
        self
    }
}

/// The price a buyer pays for `variant`: its own override, else the product's base price.
pub fn effective_price(product: &Product, variant: &Variant) -> i64 {
    unit_price(product.base_price_kobo, variant.price_kobo)
}

/// [`effective_price`] from the raw columns, for callers (cart, checkout) that load
/// only the price fields: the variant's override, else the product's base price.
pub fn unit_price(base_price_kobo: i64, variant_price_kobo: Option<i64>) -> i64 {
    variant_price_kobo.unwrap_or(base_price_kobo)
}

/// A product's category must be one of the fixed list (`GET /categories`).
async fn require_category<'e>(
    db: impl sqlx::PgExecutor<'e>,
    category: &str,
) -> Result<(), AppError> {
    if crate::catalog::categories::exists(db, category.trim()).await? {
        return Ok(());
    }
    Err(AppError::Unprocessable(
        "unknown_category",
        "Choose one of the listed categories".into(),
    ))
}

fn invalid(msg: &str) -> AppError {
    AppError::Validation(msg.to_string())
}

fn duplicate_variant() -> AppError {
    AppError::Unprocessable(
        "duplicate_variant",
        "Two variants share the same attributes".into(),
    )
}

fn variant_in_use() -> AppError {
    AppError::Conflict("variant_in_use")
}

fn validate_text(value: &str, max: usize, field: &str) -> Result<(), AppError> {
    let n = value.trim().chars().count();
    if n == 0 || n > max {
        return Err(AppError::Validation(format!(
            "{field} must be 1 to {max} characters"
        )));
    }
    Ok(())
}

fn validate_brand(brand: &Option<String>) -> Result<(), AppError> {
    match brand {
        Some(b) if b.chars().count() > 60 => Err(invalid("brand must be at most 60 characters")),
        _ => Ok(()),
    }
}

fn validate_condition(condition: &str) -> Result<(), AppError> {
    if CONDITIONS.contains(&condition) {
        Ok(())
    } else {
        Err(invalid("condition must be new, used or refurbished"))
    }
}

fn validate_photos(keys: &[String]) -> Result<(), AppError> {
    if keys.is_empty() || keys.len() > 10 {
        return Err(invalid("photo_keys must contain 1 to 10 entries"));
    }
    if keys.iter().any(|k| k.trim().is_empty()) {
        return Err(invalid("photo_keys must not contain empty entries"));
    }
    Ok(())
}

fn validate_positive(value: i64, field: &str) -> Result<(), AppError> {
    if value > 0 {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "{field} must be greater than 0"
        )))
    }
}

fn validate_price(value: i64, field: &str) -> Result<(), AppError> {
    validate_positive(value, field)?;
    if value > MAX_PRICE_KOBO {
        return Err(AppError::Validation(format!(
            "{field} must be at most {MAX_PRICE_KOBO}"
        )));
    }
    Ok(())
}

/// Trims attribute keys and values so "M" and "M " are the same variant, in Rust and in the DB.
fn normalize_variants(variants: Vec<VariantInput>) -> Vec<VariantInput> {
    variants
        .into_iter()
        .map(|mut v| {
            v.attributes = std::mem::take(&mut v.attributes)
                .into_iter()
                .map(|(k, val)| (k.trim().to_string(), val.trim().to_string()))
                .collect();
            v
        })
        .collect()
}

fn validate_dimensions(l: Option<i32>, w: Option<i32>, h: Option<i32>) -> Result<(), AppError> {
    for (v, f) in [(l, "length_cm"), (w, "width_cm"), (h, "height_cm")] {
        if let Some(v) = v {
            validate_positive(v.into(), f)?;
        }
    }
    Ok(())
}

/// Validates a full variant set; duplicates are detected here before any insert.
fn validate_variants(variants: &[VariantInput]) -> Result<(), AppError> {
    if variants.is_empty() || variants.len() > MAX_VARIANTS {
        return Err(invalid("variants must contain 1 to 50 entries"));
    }
    for v in variants {
        v.stock.validate()?;
        if let Some(p) = v.price_kobo {
            validate_price(p, "variant price_kobo")?;
        }
        if v.attributes.len() > MAX_ATTRIBUTES {
            return Err(invalid("a variant can have at most 5 attributes"));
        }
        for (k, val) in &v.attributes {
            validate_text(k, 40, "variant attribute key")?;
            validate_text(val, 40, "variant attribute value")?;
        }
    }
    if variants.len() > 1 && variants.iter().any(|v| v.attributes.is_empty()) {
        return Err(invalid(
            "a variant without attributes cannot be combined with other variants",
        ));
    }
    for (i, v) in variants.iter().enumerate() {
        if variants[..i].iter().any(|o| o.attributes == v.attributes) {
            return Err(duplicate_variant());
        }
    }
    Ok(())
}

/// Each variant's count per branch, from what the seller sent (see [`StockInput::resolve`]).
fn resolve_stock(
    variants: &[VariantInput],
    open: &[Uuid],
) -> Result<Vec<BTreeMap<Uuid, i32>>, AppError> {
    variants.iter().map(|v| v.stock.resolve(open)).collect()
}

/// A change of stock needs [`Action::EditStock`] at every branch it touches.
fn require_stock_rights(
    member: &Membership,
    touched: impl IntoIterator<Item = Uuid>,
) -> Result<(), AppError> {
    if touched
        .into_iter()
        .all(|b| member.can(Action::EditStock(b)))
    {
        Ok(())
    } else {
        Err(AppError::Forbidden("not_allowed"))
    }
}

/// Writes each variant's counts through the stock module, in variant id order (the
/// lock order for several variants).
async fn write_stock(
    tx: &mut Transaction<'_, Postgres>,
    mut writes: Vec<(Uuid, BTreeMap<Uuid, i32>)>,
) -> Result<(), AppError> {
    writes.sort_by_key(|(id, _)| *id);
    for (variant, counts) in writes {
        for (branch, n) in counts {
            stock::set_stock(tx, branch, variant, n).await?;
        }
    }
    Ok(())
}

async fn insert_variants(
    tx: &mut Transaction<'_, Postgres>,
    product_id: Uuid,
    variants: &[VariantInput],
    counts: &[BTreeMap<Uuid, i32>],
) -> Result<(), AppError> {
    let mut writes = Vec::with_capacity(variants.len());
    for (v, c) in variants.iter().zip(counts) {
        let attrs = serde_json::to_value(&v.attributes).context("serialise attributes")?;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO product_variants (product_id, attributes, stock, price_kobo, sku) \
             VALUES ($1, $2, 0, $3, $4) RETURNING id",
        )
        .bind(product_id)
        .bind(attrs)
        .bind(v.price_kobo)
        .bind(&v.sku)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db) if db.is_unique_violation() => duplicate_variant(),
            _ => AppError::Internal(anyhow::Error::new(e).context("insert variant")),
        })?;
        writes.push((id, c.clone()));
    }
    write_stock(tx, writes).await
}

/// Makes the product's variants match `variants`: those with the same attributes keep their id
/// (price and sku are updated, and the counts at the branches named), new ones are inserted,
/// and missing ones are deleted. Every branch whose stock this changes needs
/// [`Action::EditStock`] from `member`, including a deleted variant's branches.
async fn upsert_variants(
    tx: &mut Transaction<'_, Postgres>,
    member: &Membership,
    product_id: Uuid,
    variants: &[VariantInput],
    counts: &[BTreeMap<Uuid, i32>],
) -> Result<(), AppError> {
    let keep: Vec<serde_json::Value> = variants
        .iter()
        .map(|v| serde_json::to_value(&v.attributes).context("serialise attributes"))
        .collect::<Result<_, _>>()?;
    // The product row is locked by the caller for the whole diff, so concurrent replaces serialise.
    // Its variants are then locked in id order, the same order checkout uses, so a replace
    // and a checkout touching the same variants cannot deadlock.
    let current: Vec<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT id, attributes FROM product_variants \
         WHERE product_id = $1 ORDER BY id FOR UPDATE",
    )
    .bind(product_id)
    .fetch_all(&mut **tx)
    .await
    .context("lock variants")?;
    let ids: Vec<Uuid> = current.iter().map(|(id, _)| *id).collect();
    let on_hand = stock::counts(tx, &ids).await?;
    let mut touched = Vec::new();
    for (attrs, c) in keep.iter().zip(counts) {
        let id = current.iter().find(|(_, a)| a == attrs).map(|(id, _)| *id);
        for (&branch, &n) in c {
            let now = id.and_then(|id| on_hand.get(&(id, branch)).copied());
            if now.unwrap_or(0) != n {
                touched.push(branch);
            }
        }
    }
    for (id, attrs) in &current {
        if !keep.contains(attrs) {
            touched.extend(
                on_hand
                    .iter()
                    .filter(|((v, _), n)| v == id && **n > 0)
                    .map(|((_, b), _)| *b),
            );
        }
    }
    require_stock_rights(member, touched)?;
    // An ordered variant is part of an order's history and cannot be deleted.
    let in_use: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM product_variants v JOIN order_items oi ON oi.variant_id = v.id \
         WHERE v.product_id = $1 AND NOT (v.attributes = ANY($2::jsonb[])))",
    )
    .bind(product_id)
    .bind(&keep)
    .fetch_one(&mut **tx)
    .await
    .context("check ordered variants")?;
    if in_use {
        return Err(variant_in_use());
    }
    // Its branch_stock rows go with it (ON DELETE CASCADE), and so do its totals.
    sqlx::query(
        "DELETE FROM product_variants WHERE product_id = $1 AND NOT (attributes = ANY($2::jsonb[]))",
    )
    .bind(product_id)
    .bind(&keep)
    .execute(&mut **tx)
    .await
    .map_err(|e| match &e {
        // Backstop for the check above: order_items' foreign key refuses the delete.
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => variant_in_use(),
        _ => AppError::Internal(anyhow::Error::new(e).context("delete removed variants")),
    })?;
    let mut writes = Vec::with_capacity(variants.len());
    for ((v, attrs), c) in variants.iter().zip(keep).zip(counts) {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO product_variants (product_id, attributes, stock, price_kobo, sku) \
             VALUES ($1, $2, 0, $3, $4) \
             ON CONFLICT (product_id, attributes) DO UPDATE \
             SET price_kobo = EXCLUDED.price_kobo, sku = EXCLUDED.sku RETURNING id",
        )
        .bind(product_id)
        .bind(attrs)
        .bind(v.price_kobo)
        .bind(&v.sku)
        .fetch_one(&mut **tx)
        .await
        .context("upsert variant")?;
        writes.push((id, c.clone()));
    }
    // On-hand stock may never drop below what pending checkouts reserve at a branch:
    // set_stock refuses with 409 `stock_below_reserved`.
    write_stock(tx, writes).await
}

/// `owner_view`: the viewer is in the store's team, so on-hand and reserved stock are included.
async fn load_variants(
    db: &PgPool,
    product_id: Uuid,
    owner_view: bool,
) -> Result<Vec<Variant>, AppError> {
    let rows: Vec<VariantRow> = sqlx::query_as(&format!(
        "SELECT v.id, v.attributes, v.stock, v.reserved_quantity, \
                {MAX_AVAILABLE_SQL} AS available, v.price_kobo, v.sku \
         FROM product_variants v WHERE v.product_id = $1 ORDER BY v.attributes::text, v.id"
    ))
    .bind(product_id)
    .fetch_all(db)
    .await
    .context("load variants")?;
    // The team's view: each open branch's counts, by branch position.
    let mut branches: BTreeMap<Uuid, Vec<BranchCount>> = BTreeMap::new();
    if owner_view {
        let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
        let counts: Vec<(Uuid, Uuid, i32, i32)> = sqlx::query_as(
            "SELECT bs.variant_id, bs.branch_id, bs.stock, bs.reserved_quantity \
             FROM branch_stock bs JOIN store_branches b ON b.id = bs.branch_id \
             WHERE bs.variant_id = ANY($1) AND b.closed_at IS NULL ORDER BY b.position",
        )
        .bind(&ids)
        .fetch_all(db)
        .await
        .context("load branch stock")?;
        for (variant, branch_id, stock, reserved_quantity) in counts {
            branches.entry(variant).or_default().push(BranchCount {
                branch_id,
                stock,
                reserved_quantity,
            });
        }
    }
    rows.into_iter()
        .map(|r| {
            Ok(Variant {
                id: r.id,
                attributes: serde_json::from_value(r.attributes).context("decode attributes")?,
                available_stock: i32::try_from(r.available).context("available range")?,
                stock: owner_view.then_some(r.stock),
                reserved_quantity: owner_view.then_some(r.reserved_quantity),
                branches: owner_view.then(|| branches.remove(&r.id).unwrap_or_default()),
                price_kobo: r.price_kobo,
                sku: r.sku,
            })
        })
        .collect::<Result<_, AppError>>()
}

/// The product with its variants. The store's owner and team also see on-hand and
/// reserved stock; everyone else sees available stock only.
async fn view(
    db: &PgPool,
    product: Product,
    viewer: Option<Uuid>,
) -> Result<ProductView, AppError> {
    let owner_view = match viewer {
        Some(viewer) => sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM stores s WHERE s.id = $1 AND {})",
            is_member_sql("$2")
        ))
        .bind(product.store_id)
        .bind(viewer)
        .fetch_one(db)
        .await
        .context("check store team")?,
        None => false,
    };
    let variants = load_variants(db, product.id, owner_view).await?;
    Ok(ProductView {
        product,
        photo_urls: Vec::new(),
        variants,
    })
}

/// Locks a live product in a live store; anything else is NotFound. Who may act on
/// it is the caller's check, made right after (a stranger gets the same NotFound).
async fn lock_live(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Product, AppError> {
    sqlx::query_as(&format!(
        "SELECT {PRODUCT_COLUMNS} FROM products p WHERE p.id = $1 AND p.deleted_at IS NULL \
         AND EXISTS (SELECT 1 FROM stores s WHERE s.id = p.store_id AND s.deleted_at IS NULL) \
         FOR NO KEY UPDATE OF p"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .context("lock product")?
    .ok_or(AppError::NotFound)
}

/// What a buyer sees as live is a manager's: on a published product a price change
/// or a change of the variant set needs [`Action::EditLivePrices`].
async fn variants_change_live_prices(
    tx: &mut Transaction<'_, Postgres>,
    product_id: Uuid,
    variants: &[VariantInput],
) -> Result<bool, AppError> {
    let rows: Vec<(serde_json::Value, Option<i64>)> =
        sqlx::query_as("SELECT attributes, price_kobo FROM product_variants WHERE product_id = $1")
            .bind(product_id)
            .fetch_all(&mut **tx)
            .await
            .context("load variant prices")?;
    let current = rows
        .into_iter()
        .map(|(attrs, price)| {
            let attrs: BTreeMap<String, String> =
                serde_json::from_value(attrs).context("decode attributes")?;
            Ok((attrs, price))
        })
        .collect::<Result<BTreeMap<_, _>, AppError>>()?;
    let wanted: BTreeMap<BTreeMap<String, String>, Option<i64>> = variants
        .iter()
        .map(|v| (v.attributes.clone(), v.price_kobo))
        .collect();
    Ok(current != wanted)
}

pub async fn create_product(
    db: &PgPool,
    actor: Uuid,
    store_id: Uuid,
    mut input: NewProduct,
) -> Result<ProductView, AppError> {
    input.variants = normalize_variants(std::mem::take(&mut input.variants));
    validate_text(&input.name, 120, "name")?;
    validate_text(&input.description, 5000, "description")?;
    validate_text(&input.category, 60, "category")?;
    require_category(db, &input.category).await?;
    validate_brand(&input.brand)?;
    validate_condition(&input.condition)?;
    validate_photos(&input.photo_keys)?;
    validate_positive(input.weight_grams.into(), "weight_grams")?;
    validate_dimensions(input.length_cm, input.width_cm, input.height_cm)?;
    validate_price(input.base_price_kobo, "base_price_kobo")?;
    validate_variants(&input.variants)?;

    let mut tx = db.begin().await.context("begin")?;
    // Anyone in the team adds products; they start unpublished. A stranger, or a
    // deleted store, gets NotFound.
    let member = access::require(&mut tx, store_id, actor, Action::EditProducts).await?;
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM stores WHERE id = $1 AND deleted_at IS NULL FOR SHARE",
    )
    .bind(store_id)
    .fetch_optional(&mut *tx)
    .await
    .context("check store")?
    .ok_or(AppError::NotFound)?;
    // Stock is counted at a branch: putting any there needs that branch's stock right.
    let open = stock::open_branches(&mut tx, store_id).await?;
    let counts = resolve_stock(&input.variants, &open)?;
    require_stock_rights(
        &member,
        counts
            .iter()
            .flat_map(|c| c.iter().filter(|(_, n)| **n != 0).map(|(b, _)| *b)),
    )?;

    let product: Product = sqlx::query_as(&format!(
        "INSERT INTO products (store_id, name, description, category, brand, condition, photo_keys, \
                               weight_grams, length_cm, width_cm, height_cm, base_price_kobo) \
         VALUES ($1, $2, $3, $4, $5, $6::product_condition, $7, $8, $9, $10, $11, $12) RETURNING {COLUMNS}"
    ))
    .bind(store_id)
    .bind(input.name.trim())
    .bind(&input.description)
    .bind(input.category.trim())
    .bind(&input.brand)
    .bind(&input.condition)
    .bind(&input.photo_keys)
    .bind(input.weight_grams)
    .bind(input.length_cm)
    .bind(input.width_cm)
    .bind(input.height_cm)
    .bind(input.base_price_kobo)
    .fetch_one(&mut *tx)
    .await
    .context("insert product")?;
    claim(
        &mut tx,
        actor,
        Purpose::Product,
        &input.photo_keys,
        (Kind::Product, product.id),
    )
    .await?;
    insert_variants(&mut tx, product.id, &input.variants, &counts).await?;
    audit::record(
        &mut tx,
        Some(actor),
        "product.created",
        "product",
        Some(product.id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    view(db, product, Some(actor)).await
}

pub async fn update_product(
    db: &PgPool,
    actor: Uuid,
    id: Uuid,
    patch: ProductPatch,
) -> Result<ProductView, AppError> {
    let mut tx = db.begin().await.context("begin")?;
    let cur = lock_live(&mut tx, id).await?;
    // Publishing is always a manager's; so is a new price once the product is live.
    // Restating the current value is no change.
    let action = if patch.is_published.is_some_and(|p| p != cur.is_published) {
        Action::PublishProducts
    } else if cur.is_published
        && patch
            .base_price_kobo
            .is_some_and(|p| p != cur.base_price_kobo)
    {
        Action::EditLivePrices
    } else {
        Action::EditProducts
    };
    access::require(&mut tx, cur.store_id, actor, action).await?;

    let name = patch.name.unwrap_or(cur.name);
    let description = patch.description.unwrap_or(cur.description);
    let category = patch.category.unwrap_or(cur.category);
    let brand = patch.brand.or(cur.brand);
    let condition = patch.condition.unwrap_or(cur.condition);
    let held_photos = cur.photo_keys.clone();
    let photo_keys = patch.photo_keys.unwrap_or(cur.photo_keys);
    let weight = patch.weight_grams.unwrap_or(cur.weight_grams);
    let (l, w, h) = (
        patch.length_cm.or(cur.length_cm),
        patch.width_cm.or(cur.width_cm),
        patch.height_cm.or(cur.height_cm),
    );
    let base = patch.base_price_kobo.unwrap_or(cur.base_price_kobo);
    let published = patch.is_published.unwrap_or(cur.is_published);

    validate_text(&name, 120, "name")?;
    validate_text(&description, 5000, "description")?;
    validate_text(&category, 60, "category")?;
    require_category(&mut *tx, &category).await?;
    validate_brand(&brand)?;
    validate_condition(&condition)?;
    validate_photos(&photo_keys)?;
    validate_positive(weight.into(), "weight_grams")?;
    validate_dimensions(l, w, h)?;
    validate_price(base, "base_price_kobo")?;

    let product: Product = sqlx::query_as(&format!(
        "UPDATE products SET name = $2, description = $3, category = $4, brand = $5, \
                condition = $6::product_condition, photo_keys = $7, weight_grams = $8, length_cm = $9, \
                width_cm = $10, height_cm = $11, base_price_kobo = $12, is_published = $13, updated_at = now() \
         WHERE id = $1 RETURNING {COLUMNS}"
    ))
    .bind(id)
    .bind(name.trim())
    .bind(&description)
    .bind(category.trim())
    .bind(&brand)
    .bind(&condition)
    .bind(&photo_keys)
    .bind(weight)
    .bind(l)
    .bind(w)
    .bind(h)
    .bind(base)
    .bind(published)
    .fetch_one(&mut *tx)
    .await
    .context("update product")?;
    // Photos the product already held are not rechecked.
    claim(
        &mut tx,
        actor,
        Purpose::Product,
        &claim::added(&held_photos, &photo_keys),
        (Kind::Product, id),
    )
    .await?;
    audit::record(
        &mut tx,
        Some(actor),
        "product.updated",
        "product",
        Some(id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    view(db, product, Some(actor)).await
}

pub async fn replace_variants(
    db: &PgPool,
    actor: Uuid,
    id: Uuid,
    variants: Vec<VariantInput>,
) -> Result<ProductView, AppError> {
    let variants = normalize_variants(variants);
    validate_variants(&variants)?;
    let mut tx = db.begin().await.context("begin")?;
    let product = lock_live(&mut tx, id).await?;
    // A stranger is NotFound before anything about the product is read.
    let member = access::require(&mut tx, product.store_id, actor, Action::EditProducts).await?;
    if product.is_published && variants_change_live_prices(&mut tx, id, &variants).await? {
        access::require(&mut tx, product.store_id, actor, Action::EditLivePrices).await?;
    }
    let open = stock::open_branches(&mut tx, product.store_id).await?;
    let counts = resolve_stock(&variants, &open)?;
    upsert_variants(&mut tx, &member, id, &variants, &counts).await?;
    sqlx::query("UPDATE products SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .context("touch product")?;
    audit::record(
        &mut tx,
        Some(actor),
        "product.variants_replaced",
        "product",
        Some(id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    view(db, product, Some(actor)).await
}

pub async fn delete_product(db: &PgPool, actor: Uuid, id: Uuid) -> Result<(), AppError> {
    let mut tx = db.begin().await.context("begin")?;
    let product = lock_live(&mut tx, id).await?;
    access::require(&mut tx, product.store_id, actor, Action::DeleteProducts).await?;
    sqlx::query("UPDATE products SET deleted_at = now(), updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .context("soft delete product")?;
    audit::record(
        &mut tx,
        Some(actor),
        "product.deleted",
        "product",
        Some(id),
        json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(())
}

/// Public when published and in an active store; the store's team also sees its products.
pub async fn get_product(
    db: &PgPool,
    id: Uuid,
    viewer: Option<Uuid>,
) -> Result<ProductView, AppError> {
    let product: Product = sqlx::query_as(&format!(
        "SELECT {PRODUCT_COLUMNS} FROM products p WHERE p.id = $1 AND p.deleted_at IS NULL AND EXISTS ( \
           SELECT 1 FROM stores s WHERE s.id = p.store_id AND s.deleted_at IS NULL AND ( \
             {} OR (p.is_published AND s.status = 'active')))",
        is_member_sql("$2")
    ))
    .bind(id)
    .bind(viewer)
    .fetch_optional(db)
    .await
    .context("get product")?
    .ok_or(AppError::NotFound)?;
    view(db, product, viewer).await
}

/// Lists a store's products by slug. Anyone who cannot see the store gets an empty list.
pub async fn list_store_products(
    db: &PgPool,
    slug: &str,
    viewer: Option<Uuid>,
) -> Result<Vec<ProductView>, AppError> {
    let products: Vec<Product> = sqlx::query_as(&format!(
        "SELECT {PRODUCT_COLUMNS} FROM products p JOIN stores s ON s.id = p.store_id \
         WHERE s.slug = $1 AND s.deleted_at IS NULL AND p.deleted_at IS NULL \
           AND ({} OR (p.is_published AND s.status = 'active')) \
         ORDER BY p.created_at, p.id",
        is_member_sql("$2")
    ))
    .bind(slug)
    .bind(viewer)
    .fetch_all(db)
    .await
    .context("list products")?;
    let mut out = Vec::with_capacity(products.len());
    for p in products {
        out.push(view(db, p, viewer).await?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(base: i64) -> Product {
        let now = OffsetDateTime::now_utc();
        Product {
            id: Uuid::nil(),
            store_id: Uuid::nil(),
            name: "n".into(),
            description: "d".into(),
            category: "c".into(),
            brand: None,
            condition: "new".into(),
            photo_keys: vec!["k".into()],
            weight_grams: 1,
            length_cm: None,
            width_cm: None,
            height_cm: None,
            base_price_kobo: base,
            is_published: false,
            created_at: now,
            updated_at: now,
        }
    }

    fn variant(price: Option<i64>) -> Variant {
        Variant {
            id: Uuid::nil(),
            attributes: BTreeMap::new(),
            available_stock: 1,
            stock: None,
            reserved_quantity: None,
            branches: None,
            price_kobo: price,
            sku: None,
        }
    }

    #[test]
    fn effective_price_uses_override() {
        let p = product(1_200_000);
        assert_eq!(effective_price(&p, &variant(Some(1_500_000))), 1_500_000);
        assert_eq!(effective_price(&p, &variant(None)), 1_200_000);
    }
}
