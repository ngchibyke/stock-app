//! The Owner's view of the partner (`GET /admin/partner/status`) and its switches
//! (`PATCH /admin/partner`), plus the webhook. Nothing here exposes a key or a secret.

use std::collections::BTreeMap;

use axum::{Json, Router, extract::State, routing::get};
use serde::Serialize;

use super::{
    breaker::State as BreakerState,
    settings::{self, Patch},
    usage,
};
use crate::{
    AppState,
    auth::{AuthUser, Role},
    error::AppError,
    extract::AppJson,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/partner/status", get(status))
        .route("/admin/partner", axum::routing::patch(update))
        .merge(super::webhook::router())
}

#[derive(Serialize)]
struct Feature {
    name: String,
    on: bool,
}

#[derive(Serialize)]
struct Usage {
    feature: String,
    calls: i64,
    errors: i64,
}

#[derive(Serialize)]
pub struct Status {
    pub configured: bool,
    pub enabled: bool,
    /// Whether the partner answered its health check just now; `null` when not checked
    /// (a settings change makes no call).
    pub connected: Option<bool>,
    /// `closed`, `open` or `half_open`.
    pub breaker: &'static str,
    features: Vec<Feature>,
    pub monthly_call_budget: Option<i32>,
    pub calls_this_month: i64,
    usage: Vec<Usage>,
    /// A fixed code such as `unavailable` or `circuit_open`; never the partner's words.
    pub last_error: Option<&'static str>,
}

async fn build(state: &AppState, check: bool) -> Result<Status, AppError> {
    let s = settings::load(&state.db).await?;
    let configured = state.partner.configured();
    // A health check only when there is something to check and the switch is on, so an idle
    // seam makes no call at all.
    let connected = if check {
        Some(configured && s.enabled && state.partner.health().await.is_ok())
    } else {
        None
    };
    let by_feature = usage::month_by_feature(&state.db).await?;
    Ok(Status {
        configured,
        enabled: s.enabled,
        connected,
        breaker: match state.partner.breaker_state() {
            BreakerState::Closed => "closed",
            BreakerState::Open => "open",
            BreakerState::HalfOpen => "half_open",
        },
        features: s
            .features
            .iter()
            .map(|(name, on)| Feature {
                name: name.clone(),
                on: *on,
            })
            .collect(),
        monthly_call_budget: s.monthly_call_budget,
        calls_this_month: by_feature.iter().map(|u| u.1).sum(),
        usage: by_feature
            .into_iter()
            .map(|(feature, calls, errors)| Usage {
                feature,
                calls,
                errors,
            })
            .collect(),
        last_error: state.partner.last_error(),
    })
}

async fn status(State(state): State<AppState>, user: AuthUser) -> Result<Json<Status>, AppError> {
    user.require_role(Role::Owner)?;
    Ok(Json(build(&state, true).await?))
}

async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(patch): AppJson<Patch>,
) -> Result<Json<Status>, AppError> {
    user.require_role(Role::Owner)?;
    settings::update(&state.db, user.id, patch).await?;
    Ok(Json(build(&state, false).await?))
}

/// The `features` map `GET /app/config` answers: each feature the Owner has named is true
/// only while the partner is available *and* its own switch is on.
pub async fn app_features(state: &AppState) -> Result<BTreeMap<String, bool>, AppError> {
    let s = settings::load(&state.db).await?;
    let available = state.partner.available().await?;
    Ok(s.features
        .into_iter()
        .map(|(name, on)| (name, on && available))
        .collect())
}
