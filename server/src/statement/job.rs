//! The daily statement check job (see the module docs of [`crate::statement`]).
//!
//! Each tick, from 03:00 Lagos time, it looks at the three days before today and
//! checks any that has no `matched` or `mismatch` row and no try within the last
//! hour. With no statement source installed it does nothing. A day the provider
//! could not answer counts as a failed row, so the job's last success stops
//! advancing while the provider is down.

use anyhow::Context;
use time::Date;
use tokio_util::sync::CancellationToken;

use super::{CheckOutcome, check_day};
use crate::{AppState, error::AppError, escrow::jobs::Stats};

/// The job's name in `job_runs`.
pub const STATEMENT_CHECK: &str = "statement_check";
/// How many finished days back the job looks.
const DAYS_BACK: i32 = 3;
/// The Lagos hour from which yesterday's statement is expected to be complete.
const EARLIEST_HOUR: i32 = 3;

pub async fn run(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    run_for(state, cancel, EARLIEST_HOUR).await
}

/// [`run`] from Lagos hour `earliest_hour` (tests pass 0, so the clock does not matter).
pub async fn run_for(
    state: &AppState,
    cancel: &CancellationToken,
    earliest_hour: i32,
) -> Result<Stats, AppError> {
    let Some(source) = state.statements.as_deref() else {
        return Ok(Stats::default());
    };
    let days: Vec<Date> = sqlx::query_scalar(
        "SELECT d::date FROM generate_series( \
             (now() AT TIME ZONE 'Africa/Lagos')::date - $1::int, \
             (now() AT TIME ZONE 'Africa/Lagos')::date - 1, interval '1 day') AS d \
         WHERE extract(hour FROM now() AT TIME ZONE 'Africa/Lagos') >= $2 \
           AND NOT EXISTS ( \
               SELECT 1 FROM statement_checks c WHERE c.day = d::date \
                 AND (c.status IN ('matched', 'mismatch') \
                      OR c.checked_at > now() - interval '1 hour')) \
         ORDER BY d",
    )
    .bind(DAYS_BACK)
    .bind(earliest_hour)
    .fetch_all(&state.db)
    .await
    .context("find days to check")?;
    let mut stats = Stats::default();
    for day in days {
        if cancel.is_cancelled() {
            break;
        }
        match check_day(&state.db, source, day, None).await? {
            CheckOutcome::Unavailable => stats.failed += 1,
            CheckOutcome::Matched | CheckOutcome::Mismatch => stats.done += 1,
        }
    }
    Ok(stats)
}
