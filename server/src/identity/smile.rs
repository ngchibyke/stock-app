//! Smile ID adapter (Biometric KYC). Facts are from Smile ID's public documentation
//! (docs.usesmileid.com, v3 API); anything the documentation does not state, and anything not
//! yet tried against a real sandbox account, is marked **unconfirmed**.
//!
//! Flow: the server asks for a short-lived token (`POST /v3/token`) bound to this seller's
//! NIN and name; the app's Smile ID SDK captures the selfie and liveness frames and submits
//! the job straight to Smile ID, so no image reaches this server; Smile ID then calls our
//! webhook with the result, which carries the name and date of birth it read from NIMC.
//!
//! Data protection: the NIN is sent to Smile ID once, in the token request, and is never
//! stored or logged. Bodies are never logged. The API key lives in [`Secret`]. Failures carry
//! no detail ([`IdentityError`]). Nothing is retried inside the adapter: a token request is
//! sent once.
//!
//! **Unconfirmed**, to settle in the sandbox before go-live:
//! - the exact shape of the token request's `payload` field (sent here as a JSON string in
//!   the multipart form) and whether `user_id` may be chosen by us;
//! - that the webhook echoes our `user_id` in `partner_params`;
//! - the webhook signature input: HMAC-SHA256 under the API key over
//!   `Response-Timestamp` + partner id + `sid_request`, base64. The docs give no separator,
//!   so none is used;
//! - that Biometric KYC needs no email or phone beyond what is bound here;
//! - the status endpoint needs a job id we do not hold, so [`fetch`](IdentityProvider::fetch)
//!   answers `None` and the webhook is the only path to a result.

use std::{fmt, time::Duration};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use hmac::{Hmac, Mac};
use reqwest::{Url, redirect::Policy};
use serde_json::{Value, json};
use sha2::Sha256;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::{
    FailReason, IdentityError, IdentityEvent, IdentityProvider, Nin, Outcome, SdkParams,
    StartedJob, Verified,
};
use crate::payments::mono::Secret;

pub const SMILE_NAME: &str = "smile_id";
pub const SANDBOX_URL: &str = "https://testapi.smileidentity.com";
pub const LIVE_URL: &str = "https://api.smileidentity.com";
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BODY: usize = 64 * 1024;
/// Smile ID's name for the 11-digit tokenised NIN. It needs an `enterprise_id` from NIMC,
/// registered in the Smile ID dashboard. **Unconfirmed** whether `NIN_SLIP` suits us better.
const ID_TYPE: &str = "NIN_V2";

#[derive(Clone)]
pub struct SmileConfig {
    pub partner_id: String,
    pub api_key: Secret,
    pub base_url: String,
    /// Where Smile ID posts results: `https://<our host>/webhooks/identity/smile_id`.
    pub callback_url: String,
    /// Shown to the seller on Smile ID's consent record.
    pub privacy_policy_url: String,
    /// Sandbox base URL in use; the app is told so it uses the sandbox too.
    pub sandbox: bool,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    /// **Tests only** (a local mock speaks plain http). Config always sets `false`.
    pub insecure_http_for_tests: bool,
}

impl fmt::Debug for SmileConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmileConfig")
            .field("base_url", &self.base_url)
            .field("sandbox", &self.sandbox)
            .field("api_key", &"[redacted]")
            .finish_non_exhaustive()
    }
}

pub struct SmileIdentity {
    cfg: SmileConfig,
    base: Url,
    client: reqwest::Client,
}

impl SmileIdentity {
    pub fn new(cfg: SmileConfig) -> anyhow::Result<Self> {
        let base = Url::parse(&cfg.base_url).context("SMILE_ID_BASE_URL is not a URL")?;
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(cfg.connect_timeout)
            .timeout(cfg.timeout)
            .redirect(Policy::none())
            .https_only(!cfg.insecure_http_for_tests)
            .build()
            .map_err(|e| anyhow!(e.without_url()))
            .context("build Smile ID HTTP client")?;
        Ok(Self { cfg, base, client })
    }
}

/// Our attempt id as Smile ID's `user_id`. **Unconfirmed** that a chosen id is accepted.
pub fn user_id_for(reference: Uuid) -> String {
    format!("user_{}", reference.simple())
}

/// A multipart/form-data body of text fields, built by hand (no extra dependency).
fn multipart(boundary: &str, fields: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in fields {
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

/// Splits a display name into given names and last name; Smile ID wants both.
fn split_name(display_name: &str) -> (String, String) {
    let words: Vec<&str> = display_name.split_whitespace().collect();
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => (rest.join(" "), (*last).to_string()),
        Some((only, _)) => ((*only).to_string(), (*only).to_string()),
        None => (String::new(), String::new()),
    }
}

#[async_trait]
impl IdentityProvider for SmileIdentity {
    fn name(&self) -> &'static str {
        SMILE_NAME
    }

    fn sdk_params(&self) -> Option<SdkParams> {
        Some(SdkParams {
            partner_id: self.cfg.partner_id.clone(),
            id_type: ID_TYPE,
            country: "NG",
            sandbox: self.cfg.sandbox,
        })
    }

    async fn start(
        &self,
        nin: &Nin,
        reference: Uuid,
        display_name: &str,
    ) -> Result<StartedJob, IdentityError> {
        let user_id = user_id_for(reference);
        let (given_names, last_name) = split_name(display_name);
        let payload = json!({
            "country": "NG",
            "id_type": ID_TYPE,
            "id_number": nin.expose(),
            "given_names": given_names,
            "last_name": last_name,
            "callback_url": self.cfg.callback_url,
            "consent": {
                "granted": true,
                "granted_at": OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .map_err(|_| IdentityError::Unavailable)?,
                "notice_language": "en",
                "notice_privacy_policy_url": self.cfg.privacy_policy_url,
            },
        });
        let boundary = format!("sm{}", Uuid::new_v4().simple());
        let body = multipart(
            &boundary,
            &[
                ("partner_id", self.cfg.partner_id.clone()),
                ("user_id", user_id.clone()),
                ("product", "biometric_kyc".to_string()),
                ("payload", payload.to_string()),
            ],
        );
        let url = self
            .base
            .join("v3/token")
            .map_err(|_| IdentityError::Unavailable)?;
        let mut api_key = reqwest::header::HeaderValue::from_str(self.cfg.api_key.expose())
            .map_err(|_| IdentityError::Unavailable)?;
        api_key.set_sensitive(true);
        let res = self
            .client
            .post(url)
            .header("smileid-partner-id", &self.cfg.partner_id)
            .header("smileid-api-key", api_key)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .map_err(|_| IdentityError::Unavailable)?;
        if !res.status().is_success() {
            tracing::warn!(status = res.status().as_u16(), "smile id token refused");
            return Err(IdentityError::Unavailable);
        }
        let bytes = res.bytes().await.map_err(|_| IdentityError::Unavailable)?;
        if bytes.len() > MAX_RESPONSE_BODY {
            return Err(IdentityError::Unavailable);
        }
        let v: Value = serde_json::from_slice(&bytes).map_err(|_| IdentityError::Unavailable)?;
        let token = v["token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or(IdentityError::Unavailable)?;
        Ok(StartedJob {
            provider_ref: user_id,
            session_token: token.to_string(),
        })
    }

    fn parse_webhook(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<IdentityEvent, IdentityError> {
        let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        let timestamp = header("response-timestamp").ok_or(IdentityError::InvalidSignature)?;
        let given = header("response-signature")
            .and_then(|s| B64.decode(s.trim()).ok())
            .ok_or(IdentityError::InvalidSignature)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(self.cfg.api_key.expose().as_bytes())
            .map_err(|_| IdentityError::InvalidSignature)?;
        mac.update(timestamp.as_bytes());
        mac.update(self.cfg.partner_id.as_bytes());
        mac.update(b"sid_request");
        mac.verify_slice(&given)
            .map_err(|_| IdentityError::InvalidSignature)?;

        let v: Value = serde_json::from_slice(body).map_err(|_| IdentityError::Malformed)?;
        let provider_ref = v["partner_params"]["user_id"]
            .as_str()
            .ok_or(IdentityError::Malformed)?
            .to_string();
        let reason = v["reason"].as_str().unwrap_or("");
        let outcome = match v["status"].as_str().ok_or(IdentityError::Malformed)? {
            "clear" => {
                let f = &v["id_fields"];
                let full_name = f["full_name"].as_str().ok_or(IdentityError::Malformed)?;
                let dob = f["date_of_birth"]
                    .as_str()
                    .ok_or(IdentityError::Malformed)?;
                let date_of_birth =
                    Date::parse(dob, &time::format_description::well_known::Iso8601::DATE)
                        .map_err(|_| IdentityError::Malformed)?;
                Outcome::Passed(Verified {
                    full_name: full_name.to_string(),
                    date_of_birth,
                })
            }
            "block" => match reason {
                "identifier_not_found" => Outcome::Failed(FailReason::NotFound),
                "face_verification_failed" => Outcome::Failed(FailReason::FaceMismatch),
                "spoof_detected" => Outcome::Failed(FailReason::NotAlive),
                // Fraud signals and anything new: a person looks, nothing is decided here.
                _ => Outcome::Unsure,
            },
            "attention" => Outcome::Unsure,
            // `error` (service trouble) and anything unknown is not a verdict.
            _ => return Err(IdentityError::Malformed),
        };
        Ok(IdentityEvent {
            provider_ref,
            outcome,
        })
    }

    async fn fetch(&self, _provider_ref: &str) -> Result<Option<Outcome>, IdentityError> {
        // The status endpoint is keyed by a job id Smile ID issues when the app submits.
        // We never see it except in the webhook. **Unconfirmed** alternative.
        Ok(None)
    }
}
