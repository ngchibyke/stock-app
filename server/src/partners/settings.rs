//! The Owner's switches for the partner: on or off, each feature on or off, and the monthly
//! call budget. Changing them needs a note and is audited (ids and the note, nothing else).

use std::collections::BTreeMap;

use anyhow::Context;
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use super::valid_feature;
use crate::{audit, error::AppError};

const NOTE_MAX: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartnerSettings {
    pub enabled: bool,
    pub features: BTreeMap<String, bool>,
    pub monthly_call_budget: Option<i32>,
}

pub async fn load(db: &PgPool) -> Result<PartnerSettings, AppError> {
    let (enabled, features, budget): (bool, serde_json::Value, Option<i32>) = sqlx::query_as(
        "SELECT partner_enabled, partner_features, partner_monthly_call_budget \
         FROM platform_settings WHERE id",
    )
    .fetch_one(db)
    .await
    .context("load partner settings")?;
    let features = features
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b)))
                .collect()
        })
        .unwrap_or_default();
    Ok(PartnerSettings {
        enabled,
        features,
        monthly_call_budget: budget,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub enabled: Option<bool>,
    /// Features to set; a feature not named keeps its value.
    pub features: Option<BTreeMap<String, bool>>,
    /// `Some(None)` (JSON `null`) removes the cap.
    #[serde(default, deserialize_with = "double_option")]
    pub monthly_call_budget: Option<Option<i32>>,
    /// Why, 1 to 500 characters. Required.
    pub note: String,
}

fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(d)?))
}

pub async fn update(db: &PgPool, actor: Uuid, patch: Patch) -> Result<PartnerSettings, AppError> {
    let note = patch.note.trim();
    if note.is_empty() || note.chars().count() > NOTE_MAX {
        return Err(AppError::Validation(format!(
            "note must be 1 to {NOTE_MAX} characters"
        )));
    }
    if patch.enabled.is_none() && patch.features.is_none() && patch.monthly_call_budget.is_none() {
        return Err(AppError::Validation("nothing to change".into()));
    }
    if let Some(f) = &patch.features
        && (f.len() > 50 || f.keys().any(|k| !valid_feature(k)))
    {
        return Err(AppError::Validation(
            "a feature name is 1 to 40 characters: lowercase letters, digits and _".into(),
        ));
    }
    if let Some(Some(b)) = patch.monthly_call_budget
        && !(0..=100_000_000).contains(&b)
    {
        return Err(AppError::Validation(
            "monthly_call_budget is out of range".into(),
        ));
    }
    let mut tx = db.begin().await.context("begin")?;
    let before: (bool, serde_json::Value, Option<i32>) = sqlx::query_as(
        "SELECT partner_enabled, partner_features, partner_monthly_call_budget \
         FROM platform_settings WHERE id FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await
    .context("lock settings")?;
    let mut features = before.1.as_object().cloned().unwrap_or_default();
    if let Some(f) = &patch.features {
        for (k, v) in f {
            features.insert(k.clone(), serde_json::Value::Bool(*v));
        }
    }
    let budget = patch.monthly_call_budget.unwrap_or(before.2);
    sqlx::query(
        "UPDATE platform_settings SET partner_enabled = $1, partner_features = $2, \
            partner_monthly_call_budget = $3 WHERE id",
    )
    .bind(patch.enabled.unwrap_or(before.0))
    .bind(serde_json::Value::Object(features))
    .bind(budget)
    .execute(&mut *tx)
    .await
    .context("save partner settings")?;
    audit::record(
        &mut tx,
        Some(actor),
        "partner.settings_changed",
        "settings",
        None,
        serde_json::json!({ "note": note }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    load(db).await
}
