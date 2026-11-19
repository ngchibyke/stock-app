//! Owner re-poll of one payout (plan 6, task 8).
//!
//! `POST /admin/payouts/{id}/repoll` asks the provider about a payout now and
//! applies the answer through the same settlement as a webhook or the job
//! ([`settle_payout`]), so it books a bank reversal of a payout that was
//! marked successful, or settles one that is still pending. It adds nothing
//! the provider did not say. Owner only; the call itself is audited.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde_json::{Value, json};

use super::{Owner, parse_id};
use crate::{
    AppState, audit,
    error::AppError,
    escrow::release::{SettlePayoutOutcome, settle_payout},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/payouts/{id}/repoll", post(repoll))
}

async fn repoll(
    State(state): State<AppState>,
    Owner(owner): Owner,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let id = parse_id(&id)?;
    let (reference, before): (String, String) =
        sqlx::query_as("SELECT reference, status::text FROM payouts WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .context("load payout")?
            .ok_or(AppError::NotFound)?;
    let outcome = settle_payout(&state, &reference).await?;
    let (answer, status) = match outcome {
        SettlePayoutOutcome::Settled { status } => ("settled", status),
        SettlePayoutOutcome::UnknownReference => return Err(AppError::NotFound),
        SettlePayoutOutcome::RetryLater => {
            return Err(AppError::Unavailable("provider_unavailable"));
        }
        SettlePayoutOutcome::Inconclusive => ("inconclusive", before.clone()),
    };
    let mut conn = state.db.acquire().await.context("acquire")?;
    audit::record(
        &mut conn,
        Some(owner.id),
        "admin.payout_repolled",
        "payout",
        Some(id),
        json!({ "from": before, "to": status, "answer": answer }),
    )
    .await?;
    Ok(Json(
        json!({ "id": id, "status": status, "answer": answer }),
    ))
}
