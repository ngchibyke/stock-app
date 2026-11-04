//! Order routes: views, buyer cancel and buyer "received", and the seller's
//! delivery steps (self-delivery: picked up, out for delivery, delivered with
//! the buyer's handover code; courier: ready for collection).

use anyhow::Context;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::header,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    service::{self, Handover},
    views::{self, DEFAULT_LIMIT, MAX_LIMIT, OrderView, Page, StaffRole},
};
use crate::{
    AppState, audit,
    auth::{AuthUser, extractor::Role},
    error::AppError,
    escrow::{ReleaseReason, refund::refund_order, release_order},
    extract::{AppJson, AppQuery},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/orders", get(list))
        .route("/orders/{id}", get(show))
        .route("/orders/{id}/tracking", get(tracking))
        .route("/orders/{id}/cancel", post(cancel))
        .route("/orders/{id}/received", post(received))
        .route("/orders/{id}/picked-up", post(picked_up))
        .route("/orders/{id}/out-for-delivery", post(out_for_delivery))
        .route("/orders/{id}/delivered", post(delivered))
        .route("/orders/{id}/ready", post(ready))
        .route("/stores/{key}/orders", get(store_orders))
}

fn parse_id(key: &str) -> Result<Uuid, AppError> {
    key.parse().map_err(|_| AppError::NotFound)
}

#[derive(Deserialize)]
struct PageQuery {
    limit: Option<i64>,
    /// Opaque cursor from the previous page's `next_cursor`.
    before: Option<String>,
}

impl PageQuery {
    fn limit(&self) -> Result<i64, AppError> {
        let limit = self.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(AppError::Validation(format!(
                "limit must be between 1 and {MAX_LIMIT}"
            )));
        }
        Ok(limit)
    }
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    AppQuery(q): AppQuery<PageQuery>,
) -> Result<Json<Page<OrderView>>, AppError> {
    let limit = q.limit()?;
    Ok(Json(
        views::list_for_buyer(&state.db, user.id, limit, q.before.as_deref()).await?,
    ))
}

async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<OrderView>, AppError> {
    let id = parse_id(&id)?;
    let staff = match user.role {
        Role::Owner => Some(StaffRole::Owner),
        Role::Support => Some(StaffRole::Support),
        Role::User => None,
    };
    Ok(Json(
        views::get_for_viewer(&state.db, user.id, staff, id).await?,
    ))
}

/// The tracking timeline, for the order's buyer or its store owner only. It can
/// carry contact details and the handover code, so it is never cached.
async fn tracking(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let id = parse_id(&id)?;
    let view = crate::delivery::tracking::get(&state, user.id, id).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(view)))
}

async fn store_orders(
    State(state): State<AppState>,
    user: AuthUser,
    Path(key): Path<String>,
    AppQuery(q): AppQuery<PageQuery>,
) -> Result<Json<Page<OrderView>>, AppError> {
    let store_id = parse_id(&key)?;
    let limit = q.limit()?;
    Ok(Json(
        views::list_for_store(&state.db, user.id, store_id, limit, q.before.as_deref()).await?,
    ))
}

/// (status, held, has a payout) if `user` is the order's buyer; anyone else gets 404.
async fn buyer_order(
    state: &AppState,
    user: &AuthUser,
    order_id: Uuid,
) -> Result<(String, bool, bool), AppError> {
    let row: Option<(String, bool, bool)> = sqlx::query_as(
        "SELECT o.status::text, o.held_at IS NOT NULL, \
                EXISTS (SELECT 1 FROM payouts p WHERE p.order_id = o.id) \
         FROM orders o WHERE o.id = $1 AND o.buyer_id = $2",
    )
    .bind(order_id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await
    .context("load buyer order")?;
    row.ok_or(AppError::NotFound)
}

/// Route-level audit row. The engines write the authoritative rows inside their
/// own transactions (`order.refund_requested`, `order.released`), so a failure
/// here is logged (error kind only, no PII) and never fails the request.
async fn record(
    state: &AppState,
    user: &AuthUser,
    action: &str,
    order_id: Uuid,
    prior_status: &str,
) {
    let result = async {
        let mut conn = state.db.acquire().await.map_err(|_| "acquire")?;
        audit::record(
            &mut conn,
            Some(user.id),
            action,
            "order",
            Some(order_id),
            json!({ "prior_status": prior_status }),
        )
        .await
        .map_err(|_| "insert")
    }
    .await;
    if let Err(kind) = result {
        tracing::error!(action, kind, "order audit write failed");
    }
}

async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<OrderView>, AppError> {
    let id = parse_id(&id)?;
    let (status, _, _) = buyer_order(&state, &user, id).await?;
    if status != "paid" {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    // A held order can still be cancelled before pickup. Stock goes back on hand.
    match refund_order(&state, id, true).await {
        Ok(_) => {}
        // Lost a race with pickup or release.
        Err(AppError::Conflict("not_refundable" | "already_released")) => {
            return Err(AppError::Conflict("invalid_order_state"));
        }
        Err(e) => return Err(e),
    }
    record(&state, &user, "order.cancelled_by_buyer", id, &status).await;
    Ok(Json(
        views::get_for_viewer(&state.db, user.id, None, id).await?,
    ))
}

/// The body must be exactly `{"confirm": true}`.
fn confirmed(body: &[u8]) -> bool {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Confirm {
        confirm: bool,
    }
    serde_json::from_slice::<Confirm>(body).is_ok_and(|c| c.confirm)
}

async fn received(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<OrderView>, AppError> {
    let id = parse_id(&id)?;
    // Ownership first, so a stranger gets 404 whatever the body says.
    let (status, held, has_payout) = buyer_order(&state, &user, id).await?;
    // Releasing the money is irreversible: the client must say so explicitly.
    if !confirmed(&body) {
        return Err(AppError::Unprocessable(
            "confirmation_required",
            "Only confirm once you hold the item. Never confirm because a seller, rider or chat asks.".into(),
        ));
    }
    match status.as_str() {
        // Repeating the call after release changes nothing.
        "completed" => {}
        "picked_up" | "out_for_delivery" | "delivered" if has_payout => {}
        "picked_up" | "out_for_delivery" | "delivered" => {
            if held {
                return Err(AppError::Conflict("order_held"));
            }
            release_order(&state, id, ReleaseReason::BuyerConfirmed).await?;
            record(&state, &user, "order.received", id, &status).await;
        }
        _ => return Err(AppError::Conflict("invalid_order_state")),
    }
    Ok(Json(
        views::get_for_viewer(&state.db, user.id, None, id).await?,
    ))
}

/// (status, delivery method) if `user` owns the order's store; anyone else
/// (buyer, other sellers, staff) gets 404, as for every order route.
async fn seller_order(
    state: &AppState,
    user: &AuthUser,
    order_id: Uuid,
) -> Result<(String, Option<String>), AppError> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT o.status::text, o.delivery_method::text \
         FROM orders o JOIN stores s ON s.id = o.store_id \
         WHERE o.id = $1 AND s.owner_id = $2 AND o.status <> 'pending_payment'",
    )
    .bind(order_id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await
    .context("load seller order")?;
    row.ok_or(AppError::NotFound)
}

/// The store owner's order, which must be delivered by `method`; any other
/// delivery method (or none, on orders from before delivery existed) is 409.
async fn seller_order_by(
    state: &AppState,
    user: &AuthUser,
    order_id: Uuid,
    method: &str,
) -> Result<String, AppError> {
    let (status, delivery) = seller_order(state, user, order_id).await?;
    if delivery.as_deref() != Some(method) {
        return Err(AppError::Conflict("invalid_order_state"));
    }
    Ok(status)
}

async fn seller_view(
    state: &AppState,
    user: &AuthUser,
    id: Uuid,
) -> Result<Json<OrderView>, AppError> {
    Ok(Json(
        views::get_for_viewer(&state.db, user.id, None, id).await?,
    ))
}

/// Self-delivery: the seller has the parcel (`paid -> picked_up`).
async fn picked_up(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<OrderView>, AppError> {
    let id = parse_id(&id)?;
    let status = seller_order_by(&state, &user, id, "self").await?;
    service::mark_picked_up(&state.db, id, OffsetDateTime::now_utc()).await?;
    if status != "picked_up" {
        record(&state, &user, "order.seller_picked_up", id, &status).await;
    }
    seller_view(&state, &user, id).await
}

/// Self-delivery: the seller sets off (`picked_up -> out_for_delivery`); the
/// buyer's handover code is made now.
async fn out_for_delivery(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<OrderView>, AppError> {
    let id = parse_id(&id)?;
    let status = seller_order_by(&state, &user, id, "self").await?;
    let mut tx = state.db.begin().await.context("begin")?;
    service::mark_out_for_delivery(&mut tx, &state.cipher, id, OffsetDateTime::now_utc()).await?;
    tx.commit().await.context("commit")?;
    if status != "out_for_delivery" {
        record(&state, &user, "order.seller_out_for_delivery", id, &status).await;
    }
    seller_view(&state, &user, id).await
}

#[derive(Deserialize)]
struct HandoverBody {
    handover_code: String,
}

/// Self-delivery: delivered, with the buyer's handover code. Entering it is
/// the buyer's confirmation, so the money is released straight after. If the
/// release fails (e.g. the order is held), the order stays `delivered` and the
/// auto-release job covers it.
async fn delivered(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    AppJson(body): AppJson<HandoverBody>,
) -> Result<Json<OrderView>, AppError> {
    let id = parse_id(&id)?;
    seller_order_by(&state, &user, id, "self").await?;
    let outcome = service::deliver_with_code(
        &state.db,
        &state.cipher,
        user.id,
        id,
        &body.handover_code,
        OffsetDateTime::now_utc(),
    )
    .await?;
    if outcome == Handover::Delivered {
        match release_order(&state, id, ReleaseReason::BuyerConfirmed).await {
            Ok(()) => {}
            // Expected: a hold must stop the money. Auto-release picks it up after unhold.
            Err(AppError::Conflict("order_held")) => {
                tracing::info!("order held at handover; not released");
            }
            Err(e) => {
                tracing::warn!(error = %e, "release after handover failed; auto-release covers it");
            }
        }
    }
    seller_view(&state, &user, id).await
}

/// Courier: the parcel is ready for collection (`ready_at` on a `paid` order).
async fn ready(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<OrderView>, AppError> {
    let id = parse_id(&id)?;
    seller_order_by(&state, &user, id, "courier").await?;
    service::mark_ready(&state.db, user.id, id, OffsetDateTime::now_utc()).await?;
    seller_view(&state, &user, id).await
}
