//! `GET /app/config`: what a phone asks for before anything else.
//!
//! Minimum app versions (so an old build can be told to update), the terms and privacy links,
//! the support address and a maintenance notice. Everything is read from the environment at
//! startup and checked then, so a typo stops the server instead of reaching phones. The
//! answer is the same for everyone and holds nothing about the caller, so it is public and
//! cacheable.

use anyhow::{Result, bail};
use axum::{Json, Router, extract::State, http::header, response::IntoResponse, routing::get};
use serde::Serialize;
use serde_json::{Value, json};

use crate::AppState;

/// How long a phone may reuse the answer.
pub const CACHE_SECS: u32 = 300;
/// Longest link accepted.
const MAX_URL_LEN: usize = 2048;
/// Longest maintenance notice, in characters.
pub const MAX_MESSAGE_CHARS: usize = 280;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AppClientConfig {
    /// Oldest app version still allowed, per platform (`MAJOR.MINOR.PATCH`).
    pub min_version_android: Option<String>,
    pub min_version_ios: Option<String>,
    pub terms_url: Option<String>,
    pub privacy_url: Option<String>,
    pub support_email: Option<String>,
    /// A notice shown while the service is down for maintenance; `maintenance.active`
    /// is true exactly when this is set.
    pub maintenance_message: Option<String>,
}

/// A value from the environment, `None` when unset or blank.
fn present(var: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    var(name)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// `MAJOR.MINOR.PATCH`, three plain numbers.
fn version(name: &str, raw: String) -> Result<String> {
    let parts: Vec<&str> = raw.split('.').collect();
    let ok = parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 9
                && p.bytes().all(|b| b.is_ascii_digit())
                && (*p == "0" || !p.starts_with('0'))
        });
    if !ok {
        bail!("{name} must be a version like 1.2.3");
    }
    Ok(raw)
}

fn https_url(name: &str, raw: String) -> Result<String> {
    let host = raw.strip_prefix("https://").unwrap_or("");
    let host = host.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty()
        || raw.len() > MAX_URL_LEN
        || raw.contains(char::is_whitespace)
        || raw.contains(char::is_control)
    {
        bail!("{name} must be an https URL");
    }
    Ok(raw)
}

fn email(name: &str, raw: String) -> Result<String> {
    let valid = raw.len() <= 254
        && !raw.contains(char::is_whitespace)
        && raw.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && !domain.contains('@')
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        });
    if !valid {
        bail!("{name} must be an email address");
    }
    Ok(raw)
}

fn message(name: &str, raw: String) -> Result<String> {
    if raw.chars().count() > MAX_MESSAGE_CHARS || raw.contains(char::is_control) {
        bail!("{name} must be one line of at most {MAX_MESSAGE_CHARS} characters");
    }
    Ok(raw)
}

impl AppClientConfig {
    /// Reads and checks every `APP_*` setting. Unset or blank means `null` in the answer.
    pub fn from_lookup(var: &impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |name: &str| present(var, name);
        Ok(Self {
            min_version_android: get("APP_MIN_VERSION_ANDROID")
                .map(|v| version("APP_MIN_VERSION_ANDROID", v))
                .transpose()?,
            min_version_ios: get("APP_MIN_VERSION_IOS")
                .map(|v| version("APP_MIN_VERSION_IOS", v))
                .transpose()?,
            terms_url: get("APP_TERMS_URL")
                .map(|v| https_url("APP_TERMS_URL", v))
                .transpose()?,
            privacy_url: get("APP_PRIVACY_URL")
                .map(|v| https_url("APP_PRIVACY_URL", v))
                .transpose()?,
            support_email: get("APP_SUPPORT_EMAIL")
                .map(|v| email("APP_SUPPORT_EMAIL", v))
                .transpose()?,
            maintenance_message: get("APP_MAINTENANCE_MESSAGE")
                .map(|v| message("APP_MAINTENANCE_MESSAGE", v))
                .transpose()?,
        })
    }

    /// The JSON the route answers with.
    pub fn view(&self) -> Value {
        json!({
            "min_version": {
                "android": self.min_version_android,
                "ios": self.min_version_ios,
            },
            "terms_url": self.terms_url,
            "privacy_url": self.privacy_url,
            "support_email": self.support_email,
            "maintenance": {
                "active": self.maintenance_message.is_some(),
                "message": self.maintenance_message,
            },
        })
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/app/config", get(show))
}

async fn show(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(
            header::CACHE_CONTROL,
            format!("public, max-age={CACHE_SECS}"),
        )],
        Json(state.app_config.view()),
    )
}
