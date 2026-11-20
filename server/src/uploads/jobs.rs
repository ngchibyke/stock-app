//! Job 14, `purge_uploads`: an upload still `pending` or `failed` after
//! [`STALE_UPLOAD_HOURS`] is deleted with its objects. The slot was abandoned, or
//! it was refused and its raw file should not linger. A finished (`ready`) upload
//! is never touched here.

use anyhow::Context;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{service::Purpose, store::Bucket};
use crate::{
    AppState,
    error::AppError,
    escrow::jobs::{BATCH, Stats},
    limits::STALE_UPLOAD_HOURS,
};

/// Job name (`job_runs.job`).
pub const PURGE_UPLOADS: &str = "purge_uploads";

pub async fn run(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    let stale: Vec<(Uuid, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id, raw_key, key, purpose FROM uploads \
         WHERE status <> 'ready' AND created_at < now() - make_interval(hours => $1) \
         ORDER BY created_at, id LIMIT $2",
    )
    .bind(STALE_UPLOAD_HOURS)
    .bind(BATCH)
    .fetch_all(&state.db)
    .await
    .context("find stale uploads")?;
    let mut stats = Stats::default();
    for (id, raw_key, key, purpose) in stale {
        if cancel.is_cancelled() {
            break;
        }
        // Objects first: a row whose objects could not be deleted stays for the next run.
        let mut ok = state
            .objects
            .delete(Bucket::Private, &raw_key)
            .await
            .is_ok();
        if let (Some(key), Some(purpose)) = (key, Purpose::parse(&purpose)) {
            ok &= state.objects.delete(purpose.bucket(), &key).await.is_ok();
        }
        if !ok {
            tracing::warn!(
                job = PURGE_UPLOADS,
                "upload objects not deleted; retrying next run"
            );
            stats.failed += 1;
            continue;
        }
        sqlx::query("DELETE FROM uploads WHERE id = $1 AND status <> 'ready'")
            .bind(id)
            .execute(&state.db)
            .await
            .context("delete stale upload")?;
        stats.done += 1;
    }
    Ok(stats)
}
