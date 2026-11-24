//! `GET /market`: the map layout plus the stores standing on it. Public and
//! read-only; it carries only what the store directory already shows publicly
//! (no owner, phone or address).

use anyhow::Context;
use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use uuid::Uuid;

use crate::{
    AppState,
    catalog::{
        browse::{Row as ProductRow, card},
        stores::{Row, card_with_logo},
    },
    error::AppError,
    stores::routes::MaybeAuth,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/market", get(market))
        .route("/stores/{key}/room", get(room))
}

#[derive(sqlx::FromRow)]
struct Placed {
    plot_id: String,
    district: String,
    #[sqlx(flatten)]
    row: Row,
}

async fn market(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    let rows: Vec<Placed> = sqlx::query_as(
        "SELECT sp.plot_id, s.district, \
                s.id, s.slug, s.name, s.logo_key, s.rating_count, s.rating_sum, s.created_at, \
                (SELECT count(*) FROM products p WHERE p.store_id = s.id \
                    AND p.is_published AND p.deleted_at IS NULL) AS product_count \
         FROM store_plots sp JOIN stores s ON s.id = sp.store_id \
         WHERE s.status = 'active' AND s.deleted_at IS NULL",
    )
    .fetch_all(&state.db)
    .await
    .context("list market placements")?;

    // Plot file order; a plot or district the layout no longer knows is skipped.
    let mut placed: Vec<(usize, Placed)> = rows
        .into_iter()
        .filter_map(|p| Some((state.market.plot_position(&p.plot_id)?, p)))
        .collect();
    placed.sort_by_key(|(pos, _)| *pos);
    let placements: Vec<Value> = placed
        .into_iter()
        .filter_map(|(_, p)| {
            let room = state.market.room_type_for_district(&p.district)?;
            Some(json!({
                "plot": p.plot_id,
                "store_type": room.id,
                "store": card_with_logo(p.row, state.objects.as_ref()),
            }))
        })
        .collect();

    let mut body = state.market.layout_json().clone();
    body.as_object_mut()
        .context("layout json is an object")?
        .insert("placements".into(), Value::Array(placements));
    let bytes = serde_json::to_vec(&body).context("serialise market")?;

    let digest = Sha256::digest(&bytes);
    let mut etag = String::from("\"");
    for b in &digest[..16] {
        etag.push_str(&format!("{b:02x}"));
    }
    etag.push('"');

    let etag_value = HeaderValue::from_str(&etag).context("etag header")?;
    let cache = HeaderValue::from_static("public, max-age=60");
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    if matches {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [(header::ETAG, etag_value), (header::CACHE_CONTROL, cache)],
        )
            .into_response());
    }
    Ok((
        [
            (header::ETAG, etag_value),
            (header::CACHE_CONTROL, cache),
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[derive(sqlx::FromRow)]
struct RoomStore {
    district: String,
    status: String,
    owner_id: Uuid,
    #[sqlx(flatten)]
    row: Row,
}

/// The store's room: its type and the in-stock products standing on its shelves.
/// Public for an active store; a pending store shows only to its owner. Anyone
/// else, and every suspended, deleted or unknown store, gets the same 404.
async fn room(
    State(state): State<AppState>,
    MaybeAuth(viewer): MaybeAuth,
    Path(slug): Path<String>,
) -> Result<Response, AppError> {
    let not_found = || AppError::NotFoundCode("store_not_found");
    let store: RoomStore = sqlx::query_as(
        "SELECT s.district, s.status::text AS status, s.owner_id,                 s.id, s.slug, s.name, s.logo_key, s.rating_count, s.rating_sum, s.created_at,                 (SELECT count(*) FROM products p WHERE p.store_id = s.id                     AND p.is_published AND p.deleted_at IS NULL) AS product_count          FROM stores s WHERE s.slug = $1 AND s.deleted_at IS NULL",
    )
    .bind(&slug)
    .fetch_optional(&state.db)
    .await
    .context("load room store")?
    .ok_or_else(not_found)?;

    let public = match store.status.as_str() {
        "active" => true,
        "pending" if viewer.as_ref().is_some_and(|u| u.id == store.owner_id) => false,
        _ => return Err(not_found()),
    };

    // A district the layout no longer knows gets the everything-else room, then
    // the boutique, so an old store never answers an error.
    let room_type = state
        .market
        .room_type_for_district(&store.district)
        .or_else(|| state.market.room_type_for_district("everything-else"))
        .or_else(|| state.market.room_type("boutique"))
        .context("layout has no fallback room type")?;
    let slots = room_type.slots.len();

    let mut rows: Vec<ProductRow> = sqlx::query_as(
        "SELECT p.id, p.name, p.category, p.condition::text AS condition,                 p.photo_keys[1] AS photo_key, p.created_at,                 COALESCE((SELECT min(COALESCE(v.price_kobo, p.base_price_kobo))                           FROM product_variants v WHERE v.product_id = p.id),                          p.base_price_kobo) AS price_from,                 true AS in_stock,                 s.id AS store_id, s.slug AS store_slug, s.name AS store_name,                 s.rating_count, s.rating_sum          FROM products p JOIN stores s ON s.id = p.store_id          WHERE s.slug = $1 AND p.is_published AND p.deleted_at IS NULL            AND EXISTS (SELECT 1 FROM product_variants v WHERE v.product_id = p.id                          AND v.stock - v.reserved_quantity > 0)          ORDER BY p.created_at DESC, p.id          LIMIT $2",
    )
    .bind(&slug)
    .bind(slots as i64 + 1)
    .fetch_all(&state.db)
    .await
    .context("list room products")?;
    let has_more = rows.len() > slots;
    rows.truncate(slots);

    let placed: Vec<Value> = rows
        .into_iter()
        .zip(&room_type.slots)
        .map(|(r, slot)| {
            let mut product = card(r);
            product.photo_url = state.objects.public_url(&product.photo_key);
            Ok(json!({ "slot": slot.id, "product": serde_json::to_value(product)? }))
        })
        .collect::<Result<_, serde_json::Error>>()
        .context("serialise room products")?;

    let body = json!({
        "store": card_with_logo(store.row, state.objects.as_ref()),
        "store_type": room_type.json,
        "placed": placed,
        "has_more": has_more,
    });
    let cache = if public {
        "public, max-age=30"
    } else {
        "private, no-store"
    };
    Ok(([(header::CACHE_CONTROL, cache)], axum::Json(body)).into_response())
}
