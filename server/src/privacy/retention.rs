//! Retention windows (Owner settings) and the job that applies them.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{AppState, audit, error::AppError, escrow::jobs::Stats};

pub const RETENTION: &str = "retention";

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Windows {
    pub chat_retention_months: i32,
    pub records_retention_years: i32,
    pub delivery_address_retention_months: i32,
    pub notice_retention_months: i32,
    pub push_device_stale_days: i32,
    pub attempt_retention_days: i32,
    pub deletion_cooling_off_days: i32,
}

pub async fn load(db: &PgPool) -> Result<Windows, AppError> {
    Ok(sqlx::query_as(
        "SELECT chat_retention_months, records_retention_years, \
                delivery_address_retention_months, notice_retention_months, \
                push_device_stale_days, attempt_retention_days, deletion_cooling_off_days \
         FROM platform_settings WHERE id",
    )
    .fetch_one(db)
    .await
    .context("load retention windows")?)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub chat_retention_months: Option<i32>,
    pub records_retention_years: Option<i32>,
    pub delivery_address_retention_months: Option<i32>,
    pub notice_retention_months: Option<i32>,
    pub push_device_stale_days: Option<i32>,
    pub attempt_retention_days: Option<i32>,
    pub deletion_cooling_off_days: Option<i32>,
}

fn check(name: &str, value: Option<i32>, lo: i32, hi: i32) -> Result<(), AppError> {
    match value {
        Some(v) if !(lo..=hi).contains(&v) => Err(AppError::Validation(format!(
            "{name} must be between {lo} and {hi}"
        ))),
        _ => Ok(()),
    }
}

pub async fn update(db: &PgPool, actor: Uuid, patch: Patch) -> Result<Windows, AppError> {
    check("chat_retention_months", patch.chat_retention_months, 1, 120)?;
    check(
        "records_retention_years",
        patch.records_retention_years,
        1,
        30,
    )?;
    check(
        "delivery_address_retention_months",
        patch.delivery_address_retention_months,
        1,
        120,
    )?;
    check(
        "notice_retention_months",
        patch.notice_retention_months,
        1,
        60,
    )?;
    check(
        "push_device_stale_days",
        patch.push_device_stale_days,
        7,
        730,
    )?;
    check(
        "attempt_retention_days",
        patch.attempt_retention_days,
        1,
        365,
    )?;
    check(
        "deletion_cooling_off_days",
        patch.deletion_cooling_off_days,
        7,
        60,
    )?;
    let mut tx = db.begin().await.context("begin")?;
    sqlx::query(
        "UPDATE platform_settings SET \
            chat_retention_months = COALESCE($1, chat_retention_months), \
            records_retention_years = COALESCE($2, records_retention_years), \
            delivery_address_retention_months = COALESCE($3, delivery_address_retention_months), \
            notice_retention_months = COALESCE($4, notice_retention_months), \
            push_device_stale_days = COALESCE($5, push_device_stale_days), \
            attempt_retention_days = COALESCE($6, attempt_retention_days), \
            deletion_cooling_off_days = COALESCE($7, deletion_cooling_off_days) \
         WHERE id",
    )
    .bind(patch.chat_retention_months)
    .bind(patch.records_retention_years)
    .bind(patch.delivery_address_retention_months)
    .bind(patch.notice_retention_months)
    .bind(patch.push_device_stale_days)
    .bind(patch.attempt_retention_days)
    .bind(patch.deletion_cooling_off_days)
    .execute(&mut *tx)
    .await
    .context("update retention windows")?;
    audit::record(
        &mut tx,
        Some(actor),
        "privacy.retention_changed",
        "settings",
        None,
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    load(db).await
}

/// One run of the retention job: every window applied once. Counts rows removed.
pub async fn run(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    let w = load(&state.db).await?;
    let mut done: u64 = 0;
    for sql in retention_sql() {
        if cancel.is_cancelled() {
            break;
        }
        let bind = match sql.1 {
            Bind::ChatMonths => w.chat_retention_months,
            Bind::RecordsYears => w.records_retention_years,
            Bind::AddressMonths => w.delivery_address_retention_months,
            Bind::NoticeMonths => w.notice_retention_months,
            Bind::PushDays => w.push_device_stale_days,
            Bind::AttemptDays => w.attempt_retention_days,
        };
        let res = sqlx::query(sql.0)
            .bind(bind)
            .execute(&state.db)
            .await
            .context("apply retention window")?;
        done += res.rows_affected();
    }
    Ok(Stats {
        done: u32::try_from(done).unwrap_or(u32::MAX),
        failed: 0,
    })
}

enum Bind {
    ChatMonths,
    RecordsYears,
    AddressMonths,
    NoticeMonths,
    PushDays,
    AttemptDays,
}

/// Each statement takes one window as `$1`, in order. A chat's window is longer while a
/// dispute on its order is open.
fn retention_sql() -> Vec<(&'static str, Bind)> {
    vec![
        (
            // Messages, oldest window first. Held and withheld rows go with the rest.
            "DELETE FROM chat_messages m USING conversations c \
             WHERE c.id = m.conversation_id \
               AND m.created_at < now() - make_interval(months => $1) \
               AND NOT (c.order_id IS NOT NULL AND EXISTS (SELECT 1 FROM disputes d \
                        WHERE d.order_id = c.order_id AND d.status IN \
                        ('awaiting_seller', 'awaiting_return', 'awaiting_decision')))",
            Bind::ChatMonths,
        ),
        (
            "DELETE FROM moderation_flags f \
             WHERE f.created_at < now() - make_interval(months => $1) \
               AND NOT EXISTS (SELECT 1 FROM chat_messages m WHERE m.flag_id = f.id)",
            Bind::ChatMonths,
        ),
        (
            "DELETE FROM moderation_strikes s \
             WHERE s.at < now() - make_interval(months => $1) \
               AND NOT EXISTS (SELECT 1 FROM moderation_flags f WHERE f.strike_id = s.id) \
               AND NOT EXISTS (SELECT 1 FROM user_moderation u \
                               WHERE u.user_id = s.user_id AND u.banned_at IS NOT NULL)",
            Bind::ChatMonths,
        ),
        (
            // A conversation with no messages left and no live notice pointing at it.
            "UPDATE notifications SET conversation_id = NULL WHERE conversation_id IN \
               (SELECT c.id FROM conversations c WHERE c.created_at < now() - make_interval(months => $1) \
                  AND NOT EXISTS (SELECT 1 FROM chat_messages m WHERE m.conversation_id = c.id))",
            Bind::ChatMonths,
        ),
        (
            "DELETE FROM conversations c \
             WHERE c.created_at < now() - make_interval(months => $1) \
               AND NOT EXISTS (SELECT 1 FROM chat_messages m WHERE m.conversation_id = c.id) \
               AND NOT EXISTS (SELECT 1 FROM notifications n WHERE n.conversation_id = c.id)",
            Bind::ChatMonths,
        ),
        (
            // A finished order's delivery address, after its window.
            "UPDATE orders SET delivery_address_enc = NULL, delivery_address_purged_at = now() \
             WHERE delivery_address_enc IS NOT NULL \
               AND status IN ('completed', 'cancelled', 'refunded') \
               AND COALESCE(completed_at, cancelled_at, delivered_at, paid_at, created_at) \
                   < now() - make_interval(months => $1) \
               AND NOT EXISTS (SELECT 1 FROM checkouts k WHERE k.id = orders.checkout_id \
                               AND k.status = 'pending')",
            Bind::AddressMonths,
        ),
        (
            "DELETE FROM notifications WHERE created_at < now() - make_interval(months => $1)",
            Bind::NoticeMonths,
        ),
        (
            "DELETE FROM push_devices WHERE registered_at < now() - make_interval(days => $1)",
            Bind::PushDays,
        ),
        (
            "DELETE FROM sign_in_failures WHERE at < now() - make_interval(days => $1)",
            Bind::AttemptDays,
        ),
        (
            "DELETE FROM redeem_attempts WHERE at < now() - make_interval(days => $1)",
            Bind::AttemptDays,
        ),
        (
            // Statements of a finished dispute are personal data; the dispute row and its
            // decision stay as the record.
            "DELETE FROM dispute_statements s USING disputes d \
             WHERE d.id = s.dispute_id AND d.status IN ('decided', 'withdrawn') \
               AND d.opened_at < now() - make_interval(years => $1)",
            Bind::RecordsYears,
        ),
        (
            "UPDATE data_exports SET status = 'expired', body_enc = NULL \
             WHERE status = 'ready' AND expires_at < now() AND $1 > 0",
            Bind::AttemptDays,
        ),
    ]
}
