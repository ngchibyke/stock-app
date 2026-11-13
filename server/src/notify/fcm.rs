//! Firebase Cloud Messaging, HTTP v1, over raw HTTP.
//!
//! Rules this adapter keeps:
//! - **Data protection.** A push carries the fixed title and body and
//!   `data: {kind, notification_id}`, nothing else. The device token, the
//!   access token, the private key and the provider's response text are never
//!   logged or put in an error: logs carry the HTTP status and a fixed cause.
//! - **Access token.** A service-account JWT assertion (RS256) is exchanged at
//!   `token_uri` for an OAuth access token, cached until five minutes before
//!   it runs out and dropped on a 401, which is retried once.
//! - **No other retries here.** A failure is a [`PushError`], and the push
//!   loop decides what happens next.
//! - Same HTTP rules as the courier adapter: rustls, https only, no
//!   redirects, short timeouts, responses capped at 64 KiB.

use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use async_trait::async_trait;
use axum::http::HeaderValue;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio::sync::Mutex;

use super::push::{PushError, PushMessage, PushSender};
pub use crate::payments::mono::Secret;

pub const DEFAULT_BASE_URL: &str = "https://fcm.googleapis.com";
/// The only token endpoint a service-account file may name.
pub const GOOGLE_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
pub const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest response read; anything longer counts as unavailable.
pub const MAX_RESPONSE_BODY: usize = 64 * 1024;
/// How long the assertion, and so the access token, is asked to live.
const ASSERTION_SECS: i64 = 3600;
/// The cached access token is dropped this long before it runs out.
const REFRESH_MARGIN: Duration = Duration::from_secs(300);
const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

#[derive(Debug, Clone)]
pub struct FcmConfig {
    pub project_id: String,
    pub client_email: String,
    /// The service account's RSA private key (PEM).
    pub private_key: Secret,
    pub token_uri: String,
    pub base_url: String,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    /// **Tests only** (a local mock server speaks plain http). `config.rs`
    /// always sets `false`.
    pub insecure_http_for_tests: bool,
}

/// The fields read from Google's service-account JSON.
#[derive(Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    token_uri: String,
}

impl FcmConfig {
    /// Reads Google's service-account JSON file. The file is a secret: no
    /// error carries any of its content.
    pub fn from_service_account_file(path: &str) -> anyhow::Result<FcmConfig> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("FCM_SERVICE_ACCOUNT_FILE cannot be read ({})", e.kind()))?;
        let account: ServiceAccount = serde_json::from_str(&raw).map_err(|_| {
            anyhow!(
                "FCM_SERVICE_ACCOUNT_FILE is not a service-account JSON file \
                 (project_id, client_email, private_key and token_uri are required)"
            )
        })?;
        if account.token_uri != GOOGLE_TOKEN_URI {
            bail!("FCM_SERVICE_ACCOUNT_FILE: token_uri must be {GOOGLE_TOKEN_URI}");
        }
        let plain = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.@".contains(c))
        };
        if !plain(&account.project_id) || !plain(&account.client_email) {
            bail!("FCM_SERVICE_ACCOUNT_FILE: project_id or client_email is malformed");
        }
        Ok(FcmConfig {
            project_id: account.project_id,
            client_email: account.client_email,
            private_key: Secret::new(&account.private_key),
            token_uri: account.token_uri,
            base_url: DEFAULT_BASE_URL.into(),
            connect_timeout: CONNECT_TIMEOUT,
            timeout: REQUEST_TIMEOUT,
            insecure_http_for_tests: false,
        })
    }
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

struct Cached {
    header: HeaderValue,
    until: Instant,
}

pub struct FcmPush {
    cfg: FcmConfig,
    send_url: Url,
    token_url: Url,
    key: EncodingKey,
    client: reqwest::Client,
    token: Mutex<Option<Cached>>,
}

impl FcmPush {
    pub fn new(cfg: FcmConfig) -> anyhow::Result<Self> {
        let send_url = Url::parse(&format!(
            "{}/v1/projects/{}/messages:send",
            cfg.base_url.trim_end_matches('/'),
            cfg.project_id
        ))
        .context("FCM_BASE_URL is not a valid base URL")?;
        let token_url = Url::parse(&cfg.token_uri).context("FCM token_uri is not a valid URL")?;
        // Never the underlying error: it could quote the key.
        let key = EncodingKey::from_rsa_pem(cfg.private_key.expose().as_bytes())
            .map_err(|_| anyhow!("the FCM service account's private_key is not an RSA PEM key"))?;
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(cfg.connect_timeout)
            .timeout(cfg.timeout)
            .redirect(Policy::none())
            .https_only(!cfg.insecure_http_for_tests)
            .build()
            .map_err(|e| anyhow!(e.without_url()))
            .context("build FCM HTTP client")?;
        Ok(Self {
            cfg,
            send_url,
            token_url,
            key,
            client,
            token: Mutex::new(None),
        })
    }

    /// The `Authorization` header: the cached access token, or a fresh one.
    /// The lock is held across the exchange, so concurrent sends share one.
    async fn bearer(&self) -> Result<HeaderValue, PushError> {
        let mut cached = self.token.lock().await;
        if let Some(token) = cached.as_ref()
            && Instant::now() < token.until
        {
            return Ok(token.header.clone());
        }
        *cached = None;
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let claims = Claims {
            iss: &self.cfg.client_email,
            scope: SCOPE,
            aud: &self.cfg.token_uri,
            iat: now,
            exp: now + ASSERTION_SECS,
        };
        let assertion = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &self.key)
            .map_err(|_| failed("sign_assertion", None))?;
        // Both values are URL-safe as they stand: base64url, dots and colons.
        let form = format!("grant_type={GRANT_TYPE}&assertion={assertion}").replace(':', "%3A");
        let res = self
            .client
            .post(self.token_url.clone())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .await
            .map_err(|_| failed("token_transport", None))?;
        let status = res.status().as_u16();
        let raw = read_capped(res)
            .await
            .ok_or_else(|| failed("token_read", Some(status)))?;
        if status != 200 {
            return Err(failed("token_status", Some(status)));
        }
        let answer: Value =
            serde_json::from_slice(&raw).map_err(|_| failed("token_bad_json", Some(status)))?;
        let (Some(access), Some(expires_in)) = (
            answer["access_token"].as_str(),
            answer["expires_in"].as_u64(),
        ) else {
            return Err(failed("token_bad_answer", Some(status)));
        };
        let mut header = HeaderValue::from_str(&format!("Bearer {access}"))
            .map_err(|_| failed("token_bad_answer", Some(status)))?;
        header.set_sensitive(true);
        let lifetime = Duration::from_secs(expires_in).saturating_sub(REFRESH_MARGIN);
        *cached = Some(Cached {
            header: header.clone(),
            until: Instant::now() + lifetime,
        });
        Ok(header)
    }

    /// Forgets `used` if it is still the cached token (another send may have
    /// replaced it already).
    async fn forget(&self, used: &HeaderValue) {
        let mut cached = self.token.lock().await;
        if cached.as_ref().is_some_and(|t| t.header == *used) {
            *cached = None;
        }
    }

    /// One POST to `messages:send`: the status and the capped body.
    async fn post(&self, bearer: &HeaderValue, body: &Value) -> Result<(u16, Vec<u8>), PushError> {
        let res = self
            .client
            .post(self.send_url.clone())
            .header("authorization", bearer.clone())
            .json(body)
            .send()
            .await
            .map_err(|_| failed("transport", None))?;
        let status = res.status().as_u16();
        let raw = read_capped(res)
            .await
            .ok_or_else(|| failed("read", Some(status)))?;
        Ok((status, raw))
    }
}

#[async_trait]
impl PushSender for FcmPush {
    async fn send(&self, m: &PushMessage<'_>) -> Result<(), PushError> {
        let body = json!({
            "message": {
                "token": m.token,
                "notification": { "title": m.title, "body": m.body },
                "data": {
                    "kind": m.kind,
                    "notification_id": m.notification_id.to_string(),
                },
            }
        });
        let bearer = self.bearer().await?;
        let (mut status, mut raw) = self.post(&bearer, &body).await?;
        if status == 401 {
            // The token was revoked or ran out early: one fresh one, one retry.
            self.forget(&bearer).await;
            let bearer = self.bearer().await?;
            (status, raw) = self.post(&bearer, &body).await?;
        }
        match status {
            200 => Ok(()),
            // Only a 404 that names the token retires a device: a wrong
            // project id is also a 404, and must not disable every phone.
            404 if error_code(&raw).as_deref() == Some("UNREGISTERED") => {
                tracing::info!(cause = "unregistered", status, "push failed");
                Err(PushError::Unregistered)
            }
            400 | 403 => {
                tracing::warn!(cause = "rejected", status, "push failed");
                Err(PushError::Rejected)
            }
            _ => Err(failed("status", Some(status))),
        }
    }
}

/// FCM's `errorCode` from an error body, if it has one.
fn error_code(raw: &[u8]) -> Option<String> {
    let body: Value = serde_json::from_slice(raw).ok()?;
    body["error"]["details"]
        .as_array()?
        .iter()
        .find_map(|d| d["errorCode"].as_str())
        .map(str::to_string)
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

/// Logs a failure with a fixed cause, never a body or a token, and gives the
/// error.
fn failed(cause: &'static str, status: Option<u16>) -> PushError {
    tracing::warn!(cause, status, "push failed");
    PushError::Unavailable
}
