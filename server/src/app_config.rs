//! `GET /config`: what the phone app needs before anyone signs in (plan 7, S3).
//! Public, and nothing in it is secret or personal.
//!
//! - `photo_base_url`: where photos are read from; the app builds a photo's address
//!   as `<photo_base_url>/<key>`. Null when no photo storage is configured.
//! - `min_app_version`: the oldest app version the server still serves. An app older
//!   than this shows a blocking "please update" screen.
//! - `fct_only`: delivery and checkout are limited to the FCT (spec section 6), so the app
//!   explains "Coming to your city soon" outside it.
//! - `payment_return_url`: the address the payment provider sends the buyer back to; the
//!   app's payment page closes when it reaches it.

use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use crate::AppState;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub photo_base_url: Option<String>,
    pub min_app_version: String,
}

/// `major.minor.patch`, digits only: the only shape the app compares.
pub fn valid_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 6 && p.chars().all(|c| c.is_ascii_digit()))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/config", get(config))
}

async fn config(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "photo_base_url": state.app_config.photo_base_url,
        "min_app_version": state.app_config.min_app_version,
        "fct_only": true,
        "payment_return_url": state.payment_redirect_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::valid_version;

    #[test]
    fn versions_are_three_numbers() {
        for ok in ["1.0.0", "0.9.12", "10.20.30"] {
            assert!(valid_version(ok), "{ok}");
        }
        for bad in [
            "",
            "1",
            "1.0",
            "1.0.0.0",
            "1.0.x",
            "v1.0.0",
            "1..0",
            "-1.0.0",
            "1.0.0-beta",
            "1234567.0.0",
        ] {
            assert!(!valid_version(bad), "{bad}");
        }
    }
}
