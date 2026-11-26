//! Jobs for part 8. `erase_accounts` erases people whose cooling-off has ended, `build_exports`
//! builds requested exports and `retention` applies the Owner's windows.

use anyhow::Context;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::erase::{Outcome, erase_user};
use crate::{AppState, error::AppError, escrow::jobs::Stats};

pub const ERASE_ACCOUNTS: &str = "erase_accounts";
const BATCH: i64 = 20;

pub async fn run_erase(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    let due: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM deletion_requests \
         WHERE status = 'pending' AND due_at <= now() ORDER BY due_at LIMIT $1",
    )
    .bind(BATCH)
    .fetch_all(&state.db)
    .await
    .context("find due deletions")?;
    let mut stats = Stats::default();
    for user in due {
        if cancel.is_cancelled() {
            break;
        }
        match erase_user(state, user).await {
            Ok(Outcome::Erased) => stats.done += 1,
            Ok(Outcome::Deferred) => {}
            Err(e) => {
                tracing::error!(job = ERASE_ACCOUNTS, error = %e, "erasure failed; will retry");
                stats.failed += 1;
            }
        }
    }
    Ok(stats)
}
