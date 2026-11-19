//! Daily reconciliation of the payment provider's statement against the ledger
//! (plan 6, task 9).
//!
//! A [`StatementSource`] reports four totals for one day, in kobo: what the
//! provider collected from buyers, paid out to sellers, refunded, and kept in
//! fees. [`check_day`] compares them with the same four sums from
//! `ledger_entries` and stores the result in `statement_checks`. A difference
//! larger than [`TOLERANCE_KOBO`] on any figure is a `mismatch` and writes
//! `statement.mismatch` to the audit log, which the dashboard lists as an
//! alert the Owner acknowledges.
//!
//! **A day** is a calendar day in Nigeria (`Africa/Lagos`), judged by when each
//! ledger entry was posted.
//!
//! **What is not built.** No Mono adapter exists: whether Mono offers a
//! statement endpoint, and in what shape and units, is one of the open Mono
//! questions in the roadmap. Production runs with no source
//! (`AppState::statements` is `None`), and the job and routes then report that
//! plainly. The ledger side and the comparison are the part that is final; an
//! adapter only has to fill [`Totals`]. The tolerance is a constant for the same
//! reason: it should be set from real statements, then moved to
//! `platform_settings`.

pub mod fake;
pub mod job;

use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::Date;
use uuid::Uuid;

use crate::{audit, error::AppError, payments::ProviderError};

/// How far a figure may differ before the day is a mismatch.
pub const TOLERANCE_KOBO: i64 = 100;
/// Audit action written for a mismatch.
pub const MISMATCH: &str = "statement.mismatch";
/// Longest wait for the provider's statement.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// One day's figures, in kobo, from the ledger or from the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Totals {
    pub collected_kobo: i64,
    pub paid_out_kobo: i64,
    pub refunded_kobo: i64,
    pub fees_kobo: i64,
}

impl Totals {
    /// `self - other`, figure by figure (saturating: bad provider data must not panic).
    pub fn minus(&self, other: &Totals) -> Totals {
        Totals {
            collected_kobo: self.collected_kobo.saturating_sub(other.collected_kobo),
            paid_out_kobo: self.paid_out_kobo.saturating_sub(other.paid_out_kobo),
            refunded_kobo: self.refunded_kobo.saturating_sub(other.refunded_kobo),
            fees_kobo: self.fees_kobo.saturating_sub(other.fees_kobo),
        }
    }

    pub fn exceeds(&self, tolerance_kobo: i64) -> bool {
        [
            self.collected_kobo,
            self.paid_out_kobo,
            self.refunded_kobo,
            self.fees_kobo,
        ]
        .iter()
        .any(|v| v.unsigned_abs() > tolerance_kobo.unsigned_abs())
    }
}

/// A provider's statement for one day.
#[async_trait]
pub trait StatementSource: Send + Sync {
    fn name(&self) -> &'static str;
    /// Totals for the Lagos calendar day `day`.
    async fn day_totals(&self, day: Date) -> Result<Totals, ProviderError>;
}

/// What a check did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    Matched,
    Mismatch,
    /// The provider could not answer; the try is recorded.
    Unavailable,
}

impl CheckOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckOutcome::Matched => "matched",
            CheckOutcome::Mismatch => "mismatch",
            CheckOutcome::Unavailable => "unavailable",
        }
    }
}

/// The ledger's figures for `day` (a Lagos calendar day).
pub async fn ledger_totals(db: &PgPool, day: Date) -> Result<Totals, AppError> {
    let (collected, paid_out, refunded, fees): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            COALESCE(-SUM(amount_kobo) FILTER (WHERE account = 'buyer_funds'), 0)::bigint, \
            COALESCE(SUM(amount_kobo) FILTER (WHERE account = 'seller_paid'), 0)::bigint, \
            COALESCE(SUM(amount_kobo) FILTER (WHERE account = 'refunds'), 0)::bigint, \
            COALESCE(SUM(amount_kobo) FILTER (WHERE account = 'provider_fees'), 0)::bigint \
         FROM ledger_entries WHERE (at AT TIME ZONE 'Africa/Lagos')::date = $1",
    )
    .bind(day)
    .fetch_one(db)
    .await
    .context("ledger totals")?;
    Ok(Totals {
        collected_kobo: collected,
        paid_out_kobo: paid_out,
        refunded_kobo: refunded,
        fees_kobo: fees,
    })
}

/// Checks one day and stores the result. `actor` is the Owner who asked, or
/// `None` for the job. Never call inside an open transaction.
pub async fn check_day(
    db: &PgPool,
    source: &dyn StatementSource,
    day: Date,
    actor: Option<Uuid>,
) -> Result<CheckOutcome, AppError> {
    let ledger = ledger_totals(db, day).await?;
    let answer = tokio::time::timeout(CALL_TIMEOUT, source.day_totals(day))
        .await
        .unwrap_or(Err(ProviderError::Timeout));
    let (outcome, statement, diff): (CheckOutcome, Option<Totals>, Option<Totals>) = match answer {
        Ok(statement) => {
            let diff = statement.minus(&ledger);
            let outcome = if diff.exceeds(TOLERANCE_KOBO) {
                CheckOutcome::Mismatch
            } else {
                CheckOutcome::Matched
            };
            (outcome, Some(statement), Some(diff))
        }
        Err(e) => {
            tracing::warn!(provider = source.name(), error = %e, "statement not available");
            (CheckOutcome::Unavailable, None, None)
        }
    };
    let to_json = |t: &Option<Totals>| -> Option<Value> {
        t.as_ref()
            .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
    };
    let mut tx = db.begin().await.context("begin")?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO statement_checks \
            (day, source, status, ledger, statement, diff, tolerance_kobo, checked_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(day)
    .bind(source.name())
    .bind(outcome.as_str())
    .bind(json!(ledger))
    .bind(to_json(&statement))
    .bind(to_json(&diff))
    .bind(TOLERANCE_KOBO)
    .bind(actor)
    .fetch_one(&mut *tx)
    .await
    .context("store statement check")?;
    if outcome == CheckOutcome::Mismatch {
        audit::record(
            &mut tx,
            actor,
            MISMATCH,
            "statement_check",
            Some(id),
            json!({ "day": day.to_string(), "diff": diff }),
        )
        .await?;
        tracing::warn!(day = %day, "provider statement does not match the ledger");
    }
    tx.commit().await.context("commit")?;
    Ok(outcome)
}
