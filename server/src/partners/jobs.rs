//! Job `partner_events`: marks received events processed (nothing handles a kind yet, so
//! that is all) and prunes processed ones after 30 days. It never calls the partner.

use anyhow::Context;
use tokio_util::sync::CancellationToken;

use crate::{AppState, error::AppError, escrow::jobs::Stats};

pub const PARTNER_EVENTS: &str = "partner_events";

pub async fn run(state: &AppState, _cancel: &CancellationToken) -> Result<Stats, AppError> {
    let done =
        sqlx::query("UPDATE partner_events SET processed_at = now() WHERE processed_at IS NULL")
            .execute(&state.db)
            .await
            .context("mark partner events")?
            .rows_affected();
    sqlx::query("DELETE FROM partner_events WHERE processed_at < now() - interval '30 days'")
        .execute(&state.db)
        .await
        .context("prune partner events")?;
    Ok(Stats {
        done: u32::try_from(done).unwrap_or(u32::MAX),
        failed: 0,
    })
}
