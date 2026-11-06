use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Postgres, QueryBuilder};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{audit, error::AppError, limits, money::FeeSchedule, payments::ProviderRegistry};

/// Payout providers the owner may select.
const KNOWN_PAYOUT_PROVIDERS: &[&str] = &["mono"];
/// `ship_deadline_enforce_from` may not lie further ahead than this.
const MAX_ENFORCE_FROM_AHEAD_DAYS: i64 = 30;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Settings {
    pub platform_cut_bps: i32,
    pub fee_small_kobo: i64,
    pub fee_large_kobo: i64,
    pub fee_threshold_kobo: i64,
    pub stamp_duty_kobo: i64,
    pub stamp_duty_threshold_kobo: i64,
    pub enabled_methods: Vec<String>,
    pub payout_provider: String,
    pub checkout_ttl_minutes: i32,
    pub ship_deadline_hours: i32,
    pub release_after_hours: i32,
    pub backstop_days: i32,
    pub payout_settlement_buffer_hours: i32,
    /// Kill switch for the ship-deadline job (auto-refund + seller strike).
    pub jobs_ship_deadline_enabled: bool,
    /// The ship-deadline job only refunds and strikes orders paid at or after
    /// this moment, so switching it on never reaches back over a backlog.
    #[serde(with = "time::serde::rfc3339")]
    pub ship_deadline_enforce_from: OffsetDateTime,
    /// Minutes after local midnight (Africa/Lagos) at which the next-day promise rolls over.
    pub delivery_cutoff_minutes: i32,
    /// How long a delivery option shown at checkout can be used.
    pub courier_quote_ttl_minutes: i32,
    pub courier_price_tolerance_bps: i32,
    pub courier_category_id: i64,
    /// Strikes older than this no longer count.
    pub strike_window_days: i32,
    /// Active strikes at which a user is muted.
    pub mute_after_strikes: i32,
    pub mute_hours: i32,
    /// Active strikes at which the owner is asked to review a ban.
    pub owner_review_after_strikes: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

impl Settings {
    pub fn fee_schedule(&self) -> FeeSchedule {
        FeeSchedule {
            small_fee: self.fee_small_kobo,
            large_fee: self.fee_large_kobo,
            threshold: self.fee_threshold_kobo,
            stamp_duty_kobo: self.stamp_duty_kobo,
            stamp_duty_threshold: self.stamp_duty_threshold_kobo,
        }
    }
}

const SELECT: &str = "SELECT platform_cut_bps, fee_small_kobo, fee_large_kobo, \
    fee_threshold_kobo, stamp_duty_kobo, stamp_duty_threshold_kobo, enabled_methods, \
    payout_provider, checkout_ttl_minutes, ship_deadline_hours, release_after_hours, \
    backstop_days, payout_settlement_buffer_hours, jobs_ship_deadline_enabled, \
    ship_deadline_enforce_from, delivery_cutoff_minutes, courier_quote_ttl_minutes, \
    courier_price_tolerance_bps, courier_category_id, strike_window_days, \
    mute_after_strikes, mute_hours, owner_review_after_strikes, updated_at \
    FROM platform_settings WHERE id";

pub async fn load(db: &PgPool) -> Result<Settings, AppError> {
    let s = sqlx::query_as::<_, Settings>(SELECT)
        .fetch_one(db)
        .await
        .context("load platform_settings")?;
    Ok(s)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub platform_cut_bps: Option<i32>,
    pub fee_small_kobo: Option<i64>,
    pub fee_large_kobo: Option<i64>,
    pub fee_threshold_kobo: Option<i64>,
    pub stamp_duty_kobo: Option<i64>,
    pub stamp_duty_threshold_kobo: Option<i64>,
    pub enabled_methods: Option<Vec<String>>,
    pub payout_provider: Option<String>,
    pub checkout_ttl_minutes: Option<i32>,
    pub ship_deadline_hours: Option<i32>,
    pub release_after_hours: Option<i32>,
    pub backstop_days: Option<i32>,
    pub payout_settlement_buffer_hours: Option<i32>,
    pub jobs_ship_deadline_enabled: Option<bool>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ship_deadline_enforce_from: Option<OffsetDateTime>,
    pub delivery_cutoff_minutes: Option<i32>,
    pub courier_quote_ttl_minutes: Option<i32>,
    pub courier_price_tolerance_bps: Option<i32>,
    pub courier_category_id: Option<i64>,
    pub strike_window_days: Option<i32>,
    pub mute_after_strikes: Option<i32>,
    pub mute_hours: Option<i32>,
    pub owner_review_after_strikes: Option<i32>,
    /// Required: the `updated_at` the owner last read. A later edit by someone
    /// else makes this one stale (409 `stale_settings`), so two owners cannot
    /// silently overwrite each other's changes.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expected_updated_at: Option<OffsetDateTime>,
}

fn in_range<T: PartialOrd>(field: &str, v: Option<T>, lo: T, hi: T) -> Result<(), AppError> {
    match v {
        Some(v) if v < lo || v > hi => {
            Err(AppError::Validation(format!("{field} is out of range")))
        }
        _ => Ok(()),
    }
}

impl SettingsPatch {
    fn is_empty(&self) -> bool {
        self.platform_cut_bps.is_none()
            && self.fee_small_kobo.is_none()
            && self.fee_large_kobo.is_none()
            && self.fee_threshold_kobo.is_none()
            && self.stamp_duty_kobo.is_none()
            && self.stamp_duty_threshold_kobo.is_none()
            && self.enabled_methods.is_none()
            && self.payout_provider.is_none()
            && self.checkout_ttl_minutes.is_none()
            && self.ship_deadline_hours.is_none()
            && self.release_after_hours.is_none()
            && self.backstop_days.is_none()
            && self.payout_settlement_buffer_hours.is_none()
            && self.jobs_ship_deadline_enabled.is_none()
            && self.ship_deadline_enforce_from.is_none()
            && self.delivery_cutoff_minutes.is_none()
            && self.courier_quote_ttl_minutes.is_none()
            && self.courier_price_tolerance_bps.is_none()
            && self.courier_category_id.is_none()
            && self.strike_window_days.is_none()
            && self.mute_after_strikes.is_none()
            && self.mute_hours.is_none()
            && self.owner_review_after_strikes.is_none()
    }

    fn validate(&self) -> Result<(), AppError> {
        let max = limits::MAX_PRICE_KOBO;
        in_range("platform_cut_bps", self.platform_cut_bps, 0, 3000)?;
        in_range("fee_small_kobo", self.fee_small_kobo, 0, max)?;
        in_range("fee_large_kobo", self.fee_large_kobo, 0, max)?;
        in_range("fee_threshold_kobo", self.fee_threshold_kobo, 0, i64::MAX)?;
        in_range("stamp_duty_kobo", self.stamp_duty_kobo, 0, max)?;
        in_range(
            "stamp_duty_threshold_kobo",
            self.stamp_duty_threshold_kobo,
            0,
            i64::MAX,
        )?;
        in_range("checkout_ttl_minutes", self.checkout_ttl_minutes, 5, 120)?;
        in_range("ship_deadline_hours", self.ship_deadline_hours, 1, 168)?;
        in_range("release_after_hours", self.release_after_hours, 1, 720)?;
        in_range("backstop_days", self.backstop_days, 1, 90)?;
        in_range(
            "payout_settlement_buffer_hours",
            self.payout_settlement_buffer_hours,
            0,
            72,
        )?;
        in_range(
            "delivery_cutoff_minutes",
            self.delivery_cutoff_minutes,
            0,
            1439,
        )?;
        in_range(
            "courier_quote_ttl_minutes",
            self.courier_quote_ttl_minutes,
            5,
            1440,
        )?;
        in_range(
            "courier_price_tolerance_bps",
            self.courier_price_tolerance_bps,
            0,
            5000,
        )?;
        in_range("courier_category_id", self.courier_category_id, 1, i64::MAX)?;
        in_range("strike_window_days", self.strike_window_days, 1, 365)?;
        in_range("mute_after_strikes", self.mute_after_strikes, 1, 20)?;
        in_range("mute_hours", self.mute_hours, 1, 720)?;
        in_range(
            "owner_review_after_strikes",
            self.owner_review_after_strikes,
            2,
            50,
        )?;
        // Far into the future would quietly switch the job's refunds off.
        if let Some(from) = self.ship_deadline_enforce_from
            && from > OffsetDateTime::now_utc() + time::Duration::days(MAX_ENFORCE_FROM_AHEAD_DAYS)
        {
            return Err(AppError::Validation(
                "ship_deadline_enforce_from is out of range".into(),
            ));
        }
        if let Some(p) = &self.payout_provider
            && !KNOWN_PAYOUT_PROVIDERS.contains(&p.as_str())
        {
            return Err(AppError::Unprocessable(
                "unknown_payout_provider",
                "payout provider is not supported".into(),
            ));
        }
        if let Some(methods) = &self.enabled_methods {
            if methods.is_empty() {
                return Err(AppError::Validation(
                    "enabled_methods must not be empty".into(),
                ));
            }
            for (i, m) in methods.iter().enumerate() {
                // Any method of a provider compiled into this build, configured or not,
                // so the owner can enable a method before its keys exist.
                if !ProviderRegistry::is_supported_method(m) {
                    return Err(AppError::Unprocessable(
                        "unknown_payment_method",
                        "a payment method is not registered".into(),
                    ));
                }
                if methods[..i].contains(m) {
                    return Err(AppError::Validation(
                        "enabled_methods must not contain duplicates".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

pub async fn update(db: &PgPool, actor: Uuid, patch: SettingsPatch) -> Result<Settings, AppError> {
    if patch.is_empty() {
        return Err(AppError::Validation(
            "at least one setting is required".into(),
        ));
    }
    patch.validate()?;
    let expected = patch
        .expected_updated_at
        .ok_or_else(|| AppError::Validation("expected_updated_at is required".into()))?;

    // Lock the row so concurrent PATCHes serialise, refuse an edit made
    // against an older version, then validate the merged result against the
    // values being replaced.
    let mut tx = db.begin().await.context("begin")?;
    let old = sqlx::query_as::<_, Settings>(&format!("{SELECT} FOR UPDATE"))
        .fetch_one(&mut *tx)
        .await
        .context("lock platform_settings")?;
    if old.updated_at != expected {
        return Err(AppError::Conflict("stale_settings"));
    }
    // Forward-only: moving the start date back would pull a backlog of orders
    // (whose sellers could not act) into the job's refunds and strikes. Checked
    // against the locked row. A wider window takes a deliberate SQL change.
    if let Some(from) = patch.ship_deadline_enforce_from
        && from < old.ship_deadline_enforce_from
    {
        return Err(AppError::Validation(
            "ship_deadline_enforce_from cannot move earlier".into(),
        ));
    }
    let backstop = patch.backstop_days.unwrap_or(old.backstop_days);
    let release = patch.release_after_hours.unwrap_or(old.release_after_hours);
    if i64::from(backstop) * 24 <= i64::from(release) {
        return Err(AppError::Unprocessable(
            "invalid_settings",
            "backstop_days must outlast release_after_hours".into(),
        ));
    }

    let mute_after = patch.mute_after_strikes.unwrap_or(old.mute_after_strikes);
    let review_after = patch
        .owner_review_after_strikes
        .unwrap_or(old.owner_review_after_strikes);
    if review_after <= mute_after {
        return Err(AppError::Unprocessable(
            "invalid_settings",
            "owner_review_after_strikes must exceed mute_after_strikes".into(),
        ));
    }

    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("UPDATE platform_settings SET ");
    let mut changed: Vec<&'static str> = Vec::new();
    {
        let mut sep = qb.separated(", ");
        macro_rules! set {
            ($($f:ident),* $(,)?) => {$(
                if let Some(v) = patch.$f {
                    sep.push(concat!(stringify!($f), " = ")).push_bind_unseparated(v);
                    changed.push(stringify!($f));
                }
            )*};
        }
        set!(
            platform_cut_bps,
            fee_small_kobo,
            fee_large_kobo,
            fee_threshold_kobo,
            stamp_duty_kobo,
            stamp_duty_threshold_kobo,
            enabled_methods,
            payout_provider,
            checkout_ttl_minutes,
            ship_deadline_hours,
            release_after_hours,
            backstop_days,
            payout_settlement_buffer_hours,
            jobs_ship_deadline_enabled,
            ship_deadline_enforce_from,
            delivery_cutoff_minutes,
            courier_quote_ttl_minutes,
            courier_price_tolerance_bps,
            courier_category_id,
            strike_window_days,
            mute_after_strikes,
            mute_hours,
            owner_review_after_strikes,
        );
    }
    if changed.is_empty() {
        return Err(AppError::Validation(
            "at least one setting is required".into(),
        ));
    }
    qb.push(", updated_at = now() WHERE id");

    qb.build()
        .execute(&mut *tx)
        .await
        .context("update platform_settings")?;
    let s = sqlx::query_as::<_, Settings>(SELECT)
        .fetch_one(&mut *tx)
        .await
        .context("reload platform_settings")?;
    // Fees and timers are commercial configuration, not personal data: old/new values are audited.
    let (old_v, new_v) = (
        serde_json::to_value(&old).context("serialise old settings")?,
        serde_json::to_value(&s).context("serialise new settings")?,
    );
    let detail: serde_json::Map<String, serde_json::Value> = changed
        .iter()
        .filter(|f| old_v[**f] != new_v[**f])
        .map(|f| (f.to_string(), json!({ "old": old_v[*f], "new": new_v[*f] })))
        .collect();
    if !detail.is_empty() {
        audit::record(
            &mut tx,
            Some(actor),
            "settings.updated",
            "platform_settings",
            None,
            serde_json::Value::Object(detail),
        )
        .await?;
    }
    tx.commit().await.context("commit")?;
    Ok(s)
}
