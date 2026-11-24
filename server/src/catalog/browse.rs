//! Public catalog browse and search: `GET /products`.
//!
//! Only products a buyer may see appear: published, not deleted, in a store that
//! is `active` and not deleted. A suspended store's products disappear at once.
//!
//! **Search** matches each word of the query against the product's name, brand
//! and category, by substring or by trigram similarity (so "sandle" finds
//! "sandal"); a product matches when any word does, and `relevance` puts the
//! best matches first. Every value is a bound parameter, and the substring
//! wildcards in a word are escaped.
//!
//! **Paging.** `newest`, `price_asc` and `price_desc` use keyset cursors over
//! `(sort key, id)`, so products with equal prices page without repeats or gaps.
//! `relevance` pages by an offset held in the cursor, and stops at
//! [`RELEVANCE_CAP`] results: relevance has no stable key to resume from, and a
//! buyer does not read past the first couple of hundred.

use anyhow::Context;
use axum::{Json, Router, extract::State, routing::get};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, QueryBuilder};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{AppState, error::AppError, extract::AppQuery, limits::MAX_PRICE_KOBO};

pub const DEFAULT_LIMIT: i64 = 24;
pub const MAX_LIMIT: i64 = 50;
/// Longest search text, in characters.
pub const MAX_QUERY_CHARS: usize = 80;
/// Words of the search text that count; the rest are ignored.
const MAX_WORDS: usize = 6;
/// The most results reachable by paging through a relevance search.
pub const RELEVANCE_CAP: i64 = 200;
/// How alike a word must be to a name for a fuzzy match (0 to 1).
const WORD_SIMILARITY: &str = "0.45";

/// The text a search looks in. The index in migration 0040 is built on exactly
/// this expression: change both together.
const DOC: &str = "lower(p.name || ' ' || coalesce(p.brand, '') || ' ' || p.category)";

pub fn router() -> Router<AppState> {
    Router::new().route("/products", get(browse))
}

#[derive(Debug, Default, Deserialize)]
pub struct BrowseQuery {
    pub q: Option<String>,
    pub category: Option<String>,
    pub condition: Option<String>,
    pub min_price_kobo: Option<i64>,
    pub max_price_kobo: Option<i64>,
    pub store: Option<String>,
    pub sort: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct StoreBrief {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    /// To one decimal place; null with no reviews.
    pub rating_average: Option<f64>,
    pub rating_count: i32,
}

#[derive(Debug, Serialize)]
pub struct ProductCard {
    pub id: Uuid,
    pub name: String,
    pub category: String,
    pub condition: String,
    /// The cheapest a buyer can pay: the lowest variant price, or the base price.
    pub price_from_kobo: i64,
    pub photo_key: String,
    /// Where the first photo is served from; null when no public store is set up.
    pub photo_url: Option<String>,
    /// Any variant with units left (stock minus those held by pending checkouts).
    pub in_stock: bool,
    pub store: StoreBrief,
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sort {
    Newest,
    PriceAsc,
    PriceDesc,
    Relevance,
}

impl Sort {
    fn tag(self) -> &'static str {
        match self {
            Sort::Newest => "n",
            Sort::PriceAsc => "a",
            Sort::PriceDesc => "d",
            Sort::Relevance => "r",
        }
    }
}

/// Where the next page starts.
#[derive(Debug)]
enum After {
    Newest(OffsetDateTime, Uuid),
    Price(i64, Uuid),
    Offset(i64),
}

fn bad_cursor() -> AppError {
    AppError::Unprocessable("bad_cursor", "That page marker is not valid".into())
}

fn encode(sort: Sort, parts: &[String]) -> String {
    let mut raw = sort.tag().to_string();
    for p in parts {
        raw.push('|');
        raw.push_str(p);
    }
    URL_SAFE_NO_PAD.encode(raw)
}

fn decode(sort: Sort, cursor: &str) -> Result<After, AppError> {
    let raw = URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(bad_cursor)?;
    let parts: Vec<&str> = raw.split('|').collect();
    if parts.first() != Some(&sort.tag()) {
        return Err(bad_cursor());
    }
    match (sort, parts.as_slice()) {
        (Sort::Newest, [_, at, id]) => Ok(After::Newest(
            OffsetDateTime::parse(at, &Rfc3339).map_err(|_| bad_cursor())?,
            id.parse().map_err(|_| bad_cursor())?,
        )),
        (Sort::PriceAsc | Sort::PriceDesc, [_, price, id]) => Ok(After::Price(
            price.parse().map_err(|_| bad_cursor())?,
            id.parse().map_err(|_| bad_cursor())?,
        )),
        (Sort::Relevance, [_, offset]) => {
            let offset: i64 = offset.parse().map_err(|_| bad_cursor())?;
            if (0..RELEVANCE_CAP).contains(&offset) {
                Ok(After::Offset(offset))
            } else {
                Err(bad_cursor())
            }
        }
        _ => Err(bad_cursor()),
    }
}

/// The search text as lower-case words, with LIKE wildcards escaped for the
/// substring match.
struct Words {
    plain: Vec<String>,
    like: Vec<String>,
}

fn words(q: Option<&str>) -> Result<Words, AppError> {
    let q = q.map(str::trim).unwrap_or("");
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(AppError::Validation(format!(
            "q must be at most {MAX_QUERY_CHARS} characters"
        )));
    }
    let plain: Vec<String> = q
        .to_lowercase()
        .split_whitespace()
        .take(MAX_WORDS)
        .map(str::to_string)
        .collect();
    let like = plain
        .iter()
        .map(|w| {
            w.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        })
        .collect();
    Ok(Words { plain, like })
}

#[derive(sqlx::FromRow)]
pub(crate) struct Row {
    id: Uuid,
    name: String,
    category: String,
    condition: String,
    photo_key: String,
    created_at: OffsetDateTime,
    price_from: i64,
    in_stock: bool,
    store_id: Uuid,
    store_slug: String,
    store_name: String,
    rating_count: i32,
    rating_sum: i64,
}

async fn browse(
    State(state): State<AppState>,
    AppQuery(q): AppQuery<BrowseQuery>,
) -> Result<Json<Page<ProductCard>>, AppError> {
    let mut page = run(&state.db, q).await?;
    for item in &mut page.items {
        item.photo_url = state.objects.public_url(&item.photo_key);
    }
    Ok(Json(page))
}

pub async fn run(db: &PgPool, q: BrowseQuery) -> Result<Page<ProductCard>, AppError> {
    let words = words(q.q.as_deref())?;
    let limit = match q.limit {
        None => DEFAULT_LIMIT,
        Some(n) if n >= 1 => n.min(MAX_LIMIT),
        Some(_) => return Err(AppError::Validation("limit must be at least 1".into())),
    };
    for (name, v) in [
        ("min_price_kobo", q.min_price_kobo),
        ("max_price_kobo", q.max_price_kobo),
    ] {
        if v.is_some_and(|v| !(1..=MAX_PRICE_KOBO).contains(&v)) {
            return Err(AppError::Validation(format!(
                "{name} must be between 1 and {MAX_PRICE_KOBO}"
            )));
        }
    }
    if let (Some(lo), Some(hi)) = (q.min_price_kobo, q.max_price_kobo)
        && lo > hi
    {
        return Err(AppError::Validation(
            "min_price_kobo must not exceed max_price_kobo".into(),
        ));
    }
    if let Some(c) = q.condition.as_deref()
        && !["new", "used", "refurbished"].contains(&c)
    {
        return Err(AppError::Validation(
            "condition must be new, used or refurbished".into(),
        ));
    }
    let searching = !words.plain.is_empty();
    let sort = match q.sort.as_deref() {
        None => {
            if searching {
                Sort::Relevance
            } else {
                Sort::Newest
            }
        }
        Some("newest") => Sort::Newest,
        Some("price_asc") => Sort::PriceAsc,
        Some("price_desc") => Sort::PriceDesc,
        // Nothing to rank without a search: newest is the honest answer.
        Some("relevance") => {
            if searching {
                Sort::Relevance
            } else {
                Sort::Newest
            }
        }
        Some(_) => {
            return Err(AppError::Validation(
                "sort must be newest, price_asc, price_desc or relevance".into(),
            ));
        }
    };
    let after = q.cursor.as_deref().map(|c| decode(sort, c)).transpose()?;

    let mut qb = QueryBuilder::<Postgres>::new(
        "SELECT * FROM (SELECT p.id, p.name, p.category, p.condition::text AS condition, \
                p.photo_keys[1] AS photo_key, p.created_at, \
                COALESCE((SELECT min(COALESCE(v.price_kobo, p.base_price_kobo)) \
                          FROM product_variants v WHERE v.product_id = p.id), \
                         p.base_price_kobo) AS price_from, \
                EXISTS (SELECT 1 FROM product_variants v WHERE v.product_id = p.id \
                          AND v.stock - v.reserved_quantity > 0) AS in_stock, \
                s.id AS store_id, s.slug AS store_slug, s.name AS store_name, \
                s.rating_count, s.rating_sum, ",
    );
    if searching {
        qb.push("(0::float8");
        for (plain, like) in words.plain.iter().zip(&words.like) {
            qb.push(" + GREATEST(word_similarity(")
                .push_bind(plain.clone())
                .push(format!(", {DOC}), CASE WHEN {DOC} ILIKE '%' || "))
                .push_bind(like.clone())
                .push(" || '%' THEN 1.0 ELSE 0.0 END)");
        }
        qb.push(") AS score");
    } else {
        qb.push("0::float8 AS score");
    }
    qb.push(
        " FROM products p JOIN stores s ON s.id = p.store_id \
          WHERE p.is_published AND p.deleted_at IS NULL \
            AND s.status = 'active' AND s.deleted_at IS NULL",
    );
    if let Some(c) = &q.category {
        qb.push(" AND p.category = ").push_bind(c.clone());
    }
    if let Some(c) = &q.condition {
        qb.push(" AND p.condition::text = ").push_bind(c.clone());
    }
    if let Some(s) = &q.store {
        qb.push(" AND s.slug = ").push_bind(s.clone());
    }
    if searching {
        qb.push(" AND (");
        for (i, (plain, like)) in words.plain.iter().zip(&words.like).enumerate() {
            if i > 0 {
                qb.push(" OR ");
            }
            qb.push(format!("{DOC} ILIKE '%' || "))
                .push_bind(like.clone())
                .push(" || '%' OR ")
                .push_bind(plain.clone())
                .push(format!(" <% {DOC}"));
        }
        qb.push(")");
    }
    qb.push(") c WHERE TRUE");
    if let Some(lo) = q.min_price_kobo {
        qb.push(" AND c.price_from >= ").push_bind(lo);
    }
    if let Some(hi) = q.max_price_kobo {
        qb.push(" AND c.price_from <= ").push_bind(hi);
    }
    match (&after, sort) {
        (Some(After::Newest(at, id)), Sort::Newest) => {
            qb.push(" AND (c.created_at, c.id) < (")
                .push_bind(*at)
                .push(", ")
                .push_bind(*id)
                .push(")");
        }
        (Some(After::Price(p, id)), Sort::PriceAsc) => {
            qb.push(" AND (c.price_from, c.id) > (")
                .push_bind(*p)
                .push(", ")
                .push_bind(*id)
                .push(")");
        }
        (Some(After::Price(p, id)), Sort::PriceDesc) => {
            qb.push(" AND (c.price_from, c.id) < (")
                .push_bind(*p)
                .push(", ")
                .push_bind(*id)
                .push(")");
        }
        _ => {}
    }
    qb.push(match sort {
        Sort::Newest => " ORDER BY c.created_at DESC, c.id DESC",
        Sort::PriceAsc => " ORDER BY c.price_from ASC, c.id ASC",
        Sort::PriceDesc => " ORDER BY c.price_from DESC, c.id DESC",
        Sort::Relevance => " ORDER BY c.score DESC, c.created_at DESC, c.id DESC",
    });
    let offset = match after {
        Some(After::Offset(n)) => n,
        _ => 0,
    };
    // Relevance never reaches past the cap, whatever the page size.
    let take = if sort == Sort::Relevance {
        limit.min(RELEVANCE_CAP - offset)
    } else {
        limit
    };
    qb.push(" LIMIT ").push_bind(take + 1);
    if sort == Sort::Relevance {
        qb.push(" OFFSET ").push_bind(offset);
    }

    // A transaction only to scope the similarity threshold to this request.
    let mut tx = db.begin().await.context("begin")?;
    if searching {
        sqlx::query(&format!(
            "SET LOCAL pg_trgm.word_similarity_threshold = {WORD_SIMILARITY}"
        ))
        .execute(&mut *tx)
        .await
        .context("set similarity threshold")?;
    }
    let mut rows: Vec<Row> = qb
        .build_query_as()
        .fetch_all(&mut *tx)
        .await
        .context("browse products")?;
    tx.commit().await.context("commit")?;

    let more = rows.len() as i64 > take;
    rows.truncate(take as usize);
    let next_cursor = if !more {
        None
    } else {
        let last = rows.last().ok_or_else(bad_cursor)?;
        Some(match sort {
            Sort::Newest => encode(
                sort,
                &[
                    last.created_at.format(&Rfc3339).context("format time")?,
                    last.id.to_string(),
                ],
            ),
            Sort::PriceAsc | Sort::PriceDesc => {
                encode(sort, &[last.price_from.to_string(), last.id.to_string()])
            }
            Sort::Relevance => encode(sort, &[(offset + take).to_string()]),
        })
    };
    // Relevance stops at the cap: no marker for a page that would pass it.
    let next_cursor =
        next_cursor.filter(|_| sort != Sort::Relevance || offset + take < RELEVANCE_CAP);

    Ok(Page {
        items: rows.into_iter().map(card).collect(),
        next_cursor,
    })
}

/// The catalog card for a row; `photo_url` is left for the caller to fill in.
pub(crate) fn card(r: Row) -> ProductCard {
    ProductCard {
        id: r.id,
        name: r.name,
        category: r.category,
        condition: r.condition,
        price_from_kobo: r.price_from,
        photo_key: r.photo_key,
        photo_url: None,
        in_stock: r.in_stock,
        store: StoreBrief {
            id: r.store_id,
            slug: r.store_slug,
            name: r.store_name,
            rating_average: (r.rating_count > 0).then(|| {
                let average = r.rating_sum as f64 / f64::from(r.rating_count);
                (average * 10.0).round() / 10.0
            }),
            rating_count: r.rating_count,
        },
    }
}
