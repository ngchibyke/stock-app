//! `GET /ready`: what an uptime monitor polls. `/health` only says the process is up;
//! this says whether the server can do its work: the database answers, and the background
//! jobs (payouts, refunds, expiry, chat, erasure and the rest) are still running and not
//! failing every time. It answers 200 `ok`, or 503 `degraded` with what is wrong.
//!
//! It is public and cheap (two small queries) and carries only job names and booleans: no
//! person, order or amount, and no error text.

use std::time::Duration;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde_json::{Value, json};

use crate::{AppState, escrow::jobs::TICK};

/// A job that has not run for this many ticks is stale: the loop is stuck or stopped.
pub const STALE_AFTER_TICKS: u32 = 5;
/// A job that reports failures and has not succeeded for this many ticks is failing.
pub const FAILING_AFTER_TICKS: u32 = 15;
const DB_TIMEOUT: Duration = Duration::from_secs(2);

pub fn router() -> Router<AppState> {
    Router::new().route("/ready", get(ready))
}

async fn ready(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let db_ok = matches!(
        tokio::time::timeout(
            DB_TIMEOUT,
            sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.db)
        )
        .await,
        Ok(Ok(1))
    );
    let (mut stale, mut failing) = (Vec::new(), Vec::new());
    if db_ok {
        let tick = TICK.as_secs() as i64;
        let rows: Result<Vec<(String, bool, bool)>, _> = sqlx::query_as(
            "SELECT job, \
                    last_run_at < now() - make_interval(secs => $1), \
                    failed > 0 AND (last_success_at IS NULL \
                        OR last_success_at < now() - make_interval(secs => $2)) \
             FROM job_runs ORDER BY job",
        )
        .bind(tick * i64::from(STALE_AFTER_TICKS))
        .bind(tick * i64::from(FAILING_AFTER_TICKS))
        .fetch_all(&state.db)
        .await;
        match rows {
            Ok(rows) => {
                for (job, is_stale, is_failing) in rows {
                    if is_stale {
                        stale.push(job.clone());
                    }
                    if is_failing {
                        failing.push(job);
                    }
                }
            }
            Err(_) => {
                return degraded(false, stale, failing);
            }
        }
    }
    if db_ok && stale.is_empty() && failing.is_empty() {
        (StatusCode::OK, Json(json!({ "status": "ok" })))
    } else {
        degraded(db_ok, stale, failing)
    }
}

fn degraded(db_ok: bool, stale: Vec<String>, failing: Vec<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "status": "degraded",
            "database": db_ok,
            "stale_jobs": stale,
            "failing_jobs": failing,
        })),
    )
}
