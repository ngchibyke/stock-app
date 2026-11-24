//! `GET /market`: the map layout plus the stores standing on it. Public and
//! read-only; it carries only what the store directory already shows publicly
//! (no owner, phone or address).

use anyhow::Context;
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    AppState,
    catalog::stores::{Row, card_with_logo},
    error::AppError,
};

pub fn router() -> Router<AppState> {
    Router::new().route("/market", get(market))
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
