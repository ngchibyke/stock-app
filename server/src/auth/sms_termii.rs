//! Termii (<https://termii.com>), a Nigerian messaging provider, as an [`SmsProvider`].
//!
//! The request is the one in Termii's messaging API documentation: one JSON `POST` to
//! `/api/sms/send` with the API key in the body, the number in international format without
//! the plus sign, and `channel: "dnd"`. That route is the one Termii says reaches numbers on
//! the do-not-disturb list; the generic route does not, and Termii warns that using it for
//! one-time codes can get the sender ID blocked. The `dnd` route has to be switched on by
//! Termii support for the account, and the sender ID has to be approved, before it delivers.
//!
//! Rules this adapter keeps (the same as the other provider adapters):
//! - **Data protection.** The phone, the code, the API key and the provider's own text are
//!   never logged or put in an error: a failure is an [`SmsError`] and the log carries only
//!   a fixed cause and the HTTP status.
//! - **HTTP.** rustls, https only, no redirects (the key is in the body, so a redirect must
//!   never carry it elsewhere), short timeouts, a capped response body.
//! - **No retries here.** The caller decides what a failure means for the person.
//!
//! Termii does not document its error bodies, so failures are classified by status: a
//! refusal (400, 401, 403, 404, 422, or a 200 whose `code` is not `ok`) is `Rejected`; a
//! throttle, a server error, a timeout, an unreadable answer or no connection is
//! `Unavailable`. Check this against Termii's sandbox before going live.

use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use async_trait::async_trait;
use reqwest::{Url, redirect::Policy};
use serde::Serialize;
use serde_json::Value;

use super::sms::{SmsError, SmsProvider};
pub use crate::payments::mono::Secret;

pub const DEFAULT_BASE_URL: &str = "https://api.ng.termii.com";
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest response read; anything longer counts as unavailable.
pub const MAX_RESPONSE_BODY: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct TermiiConfig {
    pub api_key: Secret,
    /// The approved sender ID shown on the phone: 3 to 11 letters and digits.
    pub sender_id: String,
    pub base_url: String,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    /// **Tests only** (a local mock server speaks plain http). `config.rs` always sets `false`.
    pub insecure_http_for_tests: bool,
}

/// What Termii's `/api/sms/send` takes.
#[derive(Serialize)]
struct SendBody<'a> {
    api_key: &'a str,
    to: &'a str,
    from: &'a str,
    sms: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    channel: &'static str,
}

pub struct TermiiSms {
    cfg: TermiiConfig,
    send_url: Url,
    client: reqwest::Client,
}

impl TermiiSms {
    pub fn new(cfg: TermiiConfig) -> anyhow::Result<Self> {
        // Check the base itself: "https://" plus a path would otherwise parse with the path's
        // first word as the host.
        let base_url = cfg.base_url.trim_end_matches('/');
        let base = Url::parse(base_url).context("TERMII_BASE_URL is not a valid base URL")?;
        if base.host_str().is_none() {
            bail!("TERMII_BASE_URL has no host");
        }
        if !cfg.insecure_http_for_tests && base.scheme() != "https" {
            bail!("TERMII_BASE_URL must be an https URL");
        }
        let send_url = Url::parse(&format!("{base_url}/api/sms/send"))
            .context("TERMII_BASE_URL is not a valid base URL")?;
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(cfg.connect_timeout)
            .timeout(cfg.timeout)
            .redirect(Policy::none())
            .https_only(!cfg.insecure_http_for_tests)
            .build()
            .map_err(|e| anyhow!(e.without_url()))
            .context("build the Termii HTTP client")?;
        Ok(Self {
            cfg,
            send_url,
            client,
        })
    }
}

#[async_trait]
impl SmsProvider for TermiiSms {
    async fn send(&self, to_e164: &str, text: &str) -> Result<(), SmsError> {
        let body = SendBody {
            api_key: self.cfg.api_key.expose(),
            // Termii wants the international format without the plus sign.
            to: to_e164.trim_start_matches('+'),
            from: &self.cfg.sender_id,
            sms: text,
            kind: "plain",
            channel: "dnd",
        };
        let res = self
            .client
            .post(self.send_url.clone())
            .json(&body)
            .send()
            .await
            .map_err(|_| failed("connection", None))?;
        let status = res.status().as_u16();
        match status {
            200..=299 => {}
            429 | 500..=599 => return Err(failed("status", Some(status))),
            _ => return Err(rejected("status", Some(status))),
        }
        let Some(raw) = read_capped(res).await else {
            return Err(failed("unreadable", Some(status)));
        };
        let Ok(answer) = serde_json::from_slice::<Value>(&raw) else {
            return Err(failed("not_json", Some(status)));
        };
        if answer["code"].as_str() == Some("ok") {
            Ok(())
        } else {
            Err(rejected("not_ok", Some(status)))
        }
    }
}

/// The response body, or `None` when it cannot be read or is over the cap.
async fn read_capped(mut res: reqwest::Response) -> Option<Vec<u8>> {
    let mut raw = Vec::new();
    while let Some(chunk) = res.chunk().await.ok()? {
        if raw.len() + chunk.len() > MAX_RESPONSE_BODY {
            return None;
        }
        raw.extend_from_slice(&chunk);
    }
    Some(raw)
}

/// Logs a failure with a fixed cause, never a body, a number or a key.
fn failed(cause: &'static str, status: Option<u16>) -> SmsError {
    tracing::warn!(cause, status, "termii request failed");
    SmsError::Unavailable
}

fn rejected(cause: &'static str, status: Option<u16>) -> SmsError {
    tracing::warn!(cause, status, "termii refused the message");
    SmsError::Rejected
}
