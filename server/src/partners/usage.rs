//! Metering: every call is counted per feature and day, after it is made (never inside a
//! transaction). The monthly budget is checked before a call; the first time it is spent in
//! a month one alert is raised, as an audit row the dashboard's alerts list shows.

use anyhow::Context;
use sqlx::PgPool;

use crate::error::AppError;

/// The audit action that is the alert (listed in `admin::alerts`).
pub const BUDGET_ALERT: &str = "partner.budget_exhausted";

pub async fn record(db: &PgPool, feature: &str, failed: bool) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO partner_usage (day, feature, calls, errors) \
         VALUES ((now() AT TIME ZONE 'UTC')::date, $1, 1, $2) \
         ON CONFLICT (day, feature) DO UPDATE \
            SET calls = partner_usage.calls + 1, errors = partner_usage.errors + EXCLUDED.errors",
    )
    .bind(feature)
    .bind(i32::from(failed))
    .execute(db)
    .await
    .context("record partner usage")?;
    Ok(())
}

/// Calls made so far this calendar month (UTC), over every feature.
pub async fn month_calls(db: &PgPool) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(SUM(calls), 0)::bigint FROM partner_usage \
         WHERE day >= date_trunc('month', now() AT TIME ZONE 'UTC')::date",
    )
    .fetch_one(db)
    .await
    .context("sum partner usage")?)
}

/// This month's calls per feature, most used first.
pub async fn month_by_feature(db: &PgPool) -> Result<Vec<(String, i64, i64)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT feature, SUM(calls)::bigint, SUM(errors)::bigint FROM partner_usage \
         WHERE day >= date_trunc('month', now() AT TIME ZONE 'UTC')::date \
         GROUP BY feature ORDER BY 2 DESC, 1",
    )
    .fetch_all(db)
    .await
    .context("partner usage by feature")?)
}

/// True once this month's calls have reached `budget`.
pub async fn spent(db: &PgPool, budget: Option<i32>) -> Result<bool, AppError> {
    match budget {
        None => Ok(false),
        Some(b) => Ok(month_calls(db).await? >= i64::from(b)),
    }
}

/// One alert per month, however many calls are refused after the budget is spent.
pub async fn raise_budget_alert(db: &PgPool) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO audit_log (actor_id, action, subject_type, subject_id, detail) \
         SELECT NULL, $1, 'partner', NULL, '{}'::jsonb \
         WHERE NOT EXISTS (SELECT 1 FROM audit_log WHERE action = $1 \
                           AND at >= date_trunc('month', now() AT TIME ZONE 'UTC'))",
    )
    .bind(BUDGET_ALERT)
    .execute(db)
    .await
    .context("raise partner budget alert")?;
    Ok(())
}
