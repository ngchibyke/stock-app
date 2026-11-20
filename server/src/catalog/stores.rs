//! The public store directory: `GET /stores`.
//!
//! Only `active`, non-deleted stores are listed. `newest` pages by a keyset
//! cursor over `(created_at, id)`; `rating` pages by an offset held in the
//! cursor and stops at [`RATING_CAP`] stores, as relevance search does.

use anyhow::Context;
use axum::{Json, Router, extract::State, routing::get};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, QueryBuilder};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use super::browse::{DEFAULT_LIMIT, MAX_LIMIT, MAX_QUERY_CHARS, Page};
use crate::{AppState, error::AppError, extract::AppQuery};

/// The most stores reachable by paging through the rating order.
pub const RATING_CAP: i64 = 200;

pub fn router() -> Router<AppState> {
    Router::new().route("/stores", get(directory))
}

#[derive(Debug, Default, Deserialize)]
pub struct DirectoryQuery {
    pub q: Option<String>,
    pub sort: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct StoreCard {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub logo_key: Option<String>,
    /// To one decimal place; null with no reviews.
    pub rating_average: Option<f64>,
    pub rating_count: i32,
    /// Published products only.
    pub product_count: i64,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    slug: String,
    name: String,
    logo_key: Option<String>,
    rating_count: i32,
    rating_sum: i64,
    created_at: OffsetDateTime,
    product_count: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Newest,
    Rating,
}

fn bad_cursor() -> AppError {
    AppError::Unprocessable("bad_cursor", "That page marker is not valid".into())
}

enum After {
    Newest(OffsetDateTime, Uuid),
    Offset(i64),
}

fn decode(sort: Sort, cursor: &str) -> Result<After, AppError> {
    let raw = URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .ok_or_else(bad_cursor)?;
    let parts: Vec<&str> = raw.split('|').collect();
    match (sort, parts.as_slice()) {
        (Sort::Newest, ["sn", at, id]) => Ok(After::Newest(
            OffsetDateTime::parse(at, &Rfc3339).map_err(|_| bad_cursor())?,
            id.parse().map_err(|_| bad_cursor())?,
        )),
        (Sort::Rating, ["sr", offset]) => {
            let offset: i64 = offset.parse().map_err(|_| bad_cursor())?;
            if (0..RATING_CAP).contains(&offset) {
                Ok(After::Offset(offset))
            } else {
                Err(bad_cursor())
            }
        }
        _ => Err(bad_cursor()),
    }
}

async fn directory(
    State(state): State<AppState>,
    AppQuery(q): AppQuery<DirectoryQuery>,
) -> Result<Json<Page<StoreCard>>, AppError> {
    Ok(Json(run(&state.db, q).await?))
}

pub async fn run(db: &PgPool, q: DirectoryQuery) -> Result<Page<StoreCard>, AppError> {
    let text = q.q.as_deref().map(str::trim).unwrap_or("");
    if text.chars().count() > MAX_QUERY_CHARS {
        return Err(AppError::Validation(format!(
            "q must be at most {MAX_QUERY_CHARS} characters"
        )));
    }
    let limit = match q.limit {
        None => DEFAULT_LIMIT,
        Some(n) if n >= 1 => n.min(MAX_LIMIT),
        Some(_) => return Err(AppError::Validation("limit must be at least 1".into())),
    };
    let sort = match q.sort.as_deref() {
        None | Some("newest") => Sort::Newest,
        Some("rating") => Sort::Rating,
        Some(_) => {
            return Err(AppError::Validation("sort must be newest or rating".into()));
        }
    };
    let after = q.cursor.as_deref().map(|c| decode(sort, c)).transpose()?;

    let mut qb = QueryBuilder::<Postgres>::new(
        "SELECT s.id, s.slug, s.name, s.logo_key, s.rating_count, s.rating_sum, s.created_at, \
                (SELECT count(*) FROM products p WHERE p.store_id = s.id \
                    AND p.is_published AND p.deleted_at IS NULL) AS product_count \
         FROM stores s WHERE s.status = 'active' AND s.deleted_at IS NULL",
    );
    if !text.is_empty() {
        let like = text
            .to_lowercase()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        qb.push(" AND lower(s.name) LIKE '%' || ")
            .push_bind(like)
            .push(" || '%'");
    }
    if let (Some(After::Newest(at, id)), Sort::Newest) = (&after, sort) {
        qb.push(" AND (s.created_at, s.id) < (")
            .push_bind(*at)
            .push(", ")
            .push_bind(*id)
            .push(")");
    }
    qb.push(match sort {
        Sort::Newest => " ORDER BY s.created_at DESC, s.id DESC",
        // Reviewed stores first, best average first; ties go to the one with more reviews.
        Sort::Rating => {
            " ORDER BY (CASE WHEN s.rating_count > 0 \
                THEN s.rating_sum::float8 / s.rating_count END) DESC NULLS LAST, \
              s.rating_count DESC, s.id DESC"
        }
    });
    let offset = match after {
        Some(After::Offset(n)) => n,
        _ => 0,
    };
    let take = if sort == Sort::Rating {
        limit.min(RATING_CAP - offset)
    } else {
        limit
    };
    qb.push(" LIMIT ").push_bind(take + 1);
    if sort == Sort::Rating {
        qb.push(" OFFSET ").push_bind(offset);
    }

    let mut rows: Vec<Row> = qb
        .build_query_as()
        .fetch_all(db)
        .await
        .context("list stores")?;
    let more = rows.len() as i64 > take;
    rows.truncate(take as usize);
    let next_cursor = if !more {
        None
    } else {
        match sort {
            Sort::Newest => {
                let last = rows.last().ok_or_else(bad_cursor)?;
                Some(URL_SAFE_NO_PAD.encode(format!(
                    "sn|{}|{}",
                    last.created_at.format(&Rfc3339).context("format time")?,
                    last.id
                )))
            }
            Sort::Rating if offset + take < RATING_CAP => {
                Some(URL_SAFE_NO_PAD.encode(format!("sr|{}", offset + take)))
            }
            Sort::Rating => None,
        }
    };
    Ok(Page {
        items: rows.into_iter().map(card).collect(),
        next_cursor,
    })
}

fn card(r: Row) -> StoreCard {
    StoreCard {
        id: r.id,
        slug: r.slug,
        name: r.name,
        logo_key: r.logo_key,
        rating_average: (r.rating_count > 0).then(|| {
            let average = r.rating_sum as f64 / f64::from(r.rating_count);
            (average * 10.0).round() / 10.0
        }),
        rating_count: r.rating_count,
        product_count: r.product_count,
    }
}
