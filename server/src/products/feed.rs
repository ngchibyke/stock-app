//! The product feed, search and categories (plan 7, S1). Public: no sign-in, like
//! the product page itself.
//!
//! The feed lists published products of active stores, newest first by default,
//! with keyset paging (never offset), so a list that changes while someone scrolls
//! never repeats or skips a product. A card carries what a list needs and nothing
//! about the seller as a person.
//!
//! - `q` (at most 80 characters) matches a name or a brand as a case-insensitive
//!   substring; `%`, `_` and `\` in it are literal.
//! - `category` is a slug from `/categories`, or the product's own free-text category,
//!   ignoring case. `condition`, `min_price_kobo`, `max_price_kobo` filter as named.
//! - `sort` is `newest` (default), `price_asc` or `price_desc`, ties broken by id.
//! - `before` is the `next_cursor` of the previous page and only fits the same `sort`.
//! - A sold-out product stays in the list with `in_stock: false`; a unit held by a
//!   pending checkout counts as gone.
//! - The price is the product's base price. A variant may override it on the product
//!   page; the card shows the base.

use anyhow::Context;
use axum::{Json, extract::State};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{AppState, error::AppError, extract::AppQuery};

const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 100;
const MAX_QUERY_CHARS: usize = 80;
const CONDITIONS: [&str; 3] = ["new", "used", "refurbished"];

#[derive(Deserialize)]
pub struct FeedQuery {
    q: Option<String>,
    category: Option<String>,
    condition: Option<String>,
    min_price_kobo: Option<i64>,
    max_price_kobo: Option<i64>,
    sort: Option<String>,
    before: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sort {
    Newest,
    PriceAsc,
    PriceDesc,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct FeedItem {
    pub id: Uuid,
    pub name: String,
    pub base_price_kobo: i64,
    pub cover_photo_key: Option<String>,
    pub brand: Option<String>,
    pub category: String,
    pub condition: String,
    pub store_id: Uuid,
    pub store_name: String,
    pub store_slug: String,
    pub rating_count: i32,
    pub rating_average: Option<f64>,
    pub in_stock: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Serialize)]
pub struct FeedPage {
    pub items: Vec<FeedItem>,
    pub next_cursor: Option<String>,
}

fn invalid(msg: &str) -> AppError {
    AppError::Validation(msg.to_string())
}

fn bad_cursor() -> AppError {
    invalid("before is not a valid cursor for this sort")
}

/// `%`, `_` and `\` literal, wrapped for a substring match.
fn contains_pattern(q: &str) -> String {
    let mut p = String::from("%");
    for c in q.to_lowercase().chars() {
        if matches!(c, '%' | '_' | '\\') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

/// The position after which the next page starts.
enum Position {
    Time(OffsetDateTime, Uuid),
    Price(i64, Uuid),
}

fn encode(sort: Sort, last: &FeedItem) -> Result<String, AppError> {
    let key = match sort {
        Sort::Newest => last.created_at.format(&Rfc3339).context("format cursor")?,
        Sort::PriceAsc | Sort::PriceDesc => last.base_price_kobo.to_string(),
    };
    Ok(URL_SAFE_NO_PAD.encode(format!("{key}|{}", last.id)))
}

fn decode(sort: Sort, cursor: &str) -> Result<Position, AppError> {
    let raw = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| bad_cursor())?;
    let raw = String::from_utf8(raw).map_err(|_| bad_cursor())?;
    let (key, id) = raw.split_once('|').ok_or_else(bad_cursor)?;
    let id: Uuid = id.parse().map_err(|_| bad_cursor())?;
    Ok(match sort {
        Sort::Newest => Position::Time(
            OffsetDateTime::parse(key, &Rfc3339).map_err(|_| bad_cursor())?,
            id,
        ),
        Sort::PriceAsc | Sort::PriceDesc => {
            Position::Price(key.parse().map_err(|_| bad_cursor())?, id)
        }
    })
}

pub async fn feed(
    State(state): State<AppState>,
    AppQuery(q): AppQuery<FeedQuery>,
) -> Result<Json<FeedPage>, AppError> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(invalid(&format!("limit must be between 1 and {MAX_LIMIT}")));
    }
    let sort = match q.sort.as_deref() {
        None | Some("newest") => Sort::Newest,
        Some("price_asc") => Sort::PriceAsc,
        Some("price_desc") => Sort::PriceDesc,
        Some(_) => return Err(invalid("sort must be newest, price_asc or price_desc")),
    };
    if let Some(c) = q.condition.as_deref()
        && !CONDITIONS.contains(&c)
    {
        return Err(invalid("condition must be new, used or refurbished"));
    }
    for (name, v) in [
        ("min_price_kobo", q.min_price_kobo),
        ("max_price_kobo", q.max_price_kobo),
    ] {
        if matches!(v, Some(n) if n < 0) {
            return Err(invalid(&format!("{name} cannot be negative")));
        }
    }
    if let (Some(lo), Some(hi)) = (q.min_price_kobo, q.max_price_kobo)
        && lo > hi
    {
        return Err(invalid("min_price_kobo cannot be more than max_price_kobo"));
    }
    let text = q.q.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if matches!(text, Some(t) if t.chars().count() > MAX_QUERY_CHARS) {
        return Err(invalid(&format!(
            "q must be at most {MAX_QUERY_CHARS} characters"
        )));
    }
    let pattern = text.map(contains_pattern);
    let category = q
        .category
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_lowercase);
    let position = q.before.as_deref().map(|c| decode(sort, c)).transpose()?;

    let mut items = query(
        &state.db,
        sort,
        pattern,
        category,
        q.condition,
        q.min_price_kobo,
        q.max_price_kobo,
        position,
        limit + 1,
    )
    .await?;
    let more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    let next_cursor = match (more, items.last()) {
        (true, Some(last)) => Some(encode(sort, last)?),
        _ => None,
    };
    Ok(Json(FeedPage { items, next_cursor }))
}

#[allow(clippy::too_many_arguments)]
async fn query(
    db: &PgPool,
    sort: Sort,
    pattern: Option<String>,
    category: Option<String>,
    condition: Option<String>,
    min_price: Option<i64>,
    max_price: Option<i64>,
    position: Option<Position>,
    take: i64,
) -> Result<Vec<FeedItem>, AppError> {
    // The order and the keyset condition go together; both are fixed text, never input.
    let (after, order) = match sort {
        Sort::Newest => (
            "($7::timestamptz IS NULL OR (p.created_at, p.id) < ($7, $8))",
            "p.created_at DESC, p.id DESC",
        ),
        Sort::PriceAsc => (
            "($9::bigint IS NULL OR (p.base_price_kobo, p.id) > ($9, $8))",
            "p.base_price_kobo ASC, p.id ASC",
        ),
        Sort::PriceDesc => (
            "($9::bigint IS NULL OR (p.base_price_kobo, p.id) < ($9, $8))",
            "p.base_price_kobo DESC, p.id DESC",
        ),
    };
    let (at, price, id) = match position {
        Some(Position::Time(t, id)) => (Some(t), None, Some(id)),
        Some(Position::Price(p, id)) => (None, Some(p), Some(id)),
        None => (None, None, None),
    };
    let sql = format!(
        "SELECT p.id, p.name, p.base_price_kobo, p.photo_keys[1] AS cover_photo_key, p.brand, \
                p.category, p.condition::text AS condition, \
                s.id AS store_id, s.name AS store_name, s.slug AS store_slug, \
                s.rating_count, \
                CASE WHEN s.rating_count > 0 THEN s.rating_sum::float8 / s.rating_count END \
                    AS rating_average, \
                EXISTS (SELECT 1 FROM product_variants v \
                        WHERE v.product_id = p.id AND v.stock > v.reserved_quantity) AS in_stock, \
                p.created_at \
         FROM products p JOIN stores s ON s.id = p.store_id \
         WHERE p.deleted_at IS NULL AND p.is_published \
           AND s.deleted_at IS NULL AND s.status = 'active' \
           AND ($1::text IS NULL \
                OR lower(p.name) LIKE $1 ESCAPE '\\' OR lower(p.brand) LIKE $1 ESCAPE '\\') \
           AND ($2::text IS NULL OR lower(p.category) = $2 \
                OR lower(p.category) IN (SELECT lower(c.name) FROM categories c WHERE c.slug = $2)) \
           AND ($3::text IS NULL OR p.condition::text = $3) \
           AND ($4::bigint IS NULL OR p.base_price_kobo >= $4) \
           AND ($5::bigint IS NULL OR p.base_price_kobo <= $5) \
           AND {after} \
         ORDER BY {order} LIMIT $6"
    );
    // $7 (time), $8 (id) and $9 (price) are the keyset position; each sort uses the ones it needs.
    let rows = sqlx::query_as(&sql)
        .bind(pattern)
        .bind(category)
        .bind(condition)
        .bind(min_price)
        .bind(max_price)
        .bind(take)
        .bind(at)
        .bind(id)
        .bind(price)
        .fetch_all(db)
        .await
        .context("product feed")?;
    Ok(rows)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Category {
    pub slug: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct CategoryList {
    pub items: Vec<Category>,
}

pub async fn categories(State(state): State<AppState>) -> Result<Json<CategoryList>, AppError> {
    let items = sqlx::query_as("SELECT slug, name FROM categories ORDER BY position")
        .fetch_all(&state.db)
        .await
        .context("list categories")?;
    Ok(Json(CategoryList { items }))
}
