//! The low-level partner API: https only, no redirects, a short connect and a total
//! timeout, a capped response, one retry for an idempotent `GET` and never for a `POST`.
//! Errors carry a fixed code and never the partner's text, a URL or the key.

use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use reqwest::{Method, Url, header::HeaderValue, redirect::Policy};
use serde_json::Value;

use super::{PartnerConfig, PartnerError};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_RESPONSE_BODY: usize = 1 << 20;

/// The only fields an outbound payload may carry, at any depth: our opaque store id, item
/// ids and barcodes, prices in integer kobo and timestamps. Never a name, phone, address,
/// chat text, statement, token or order contents.
pub const ALLOWED_OUTBOUND_FIELDS: &[&str] = &[
    "store_ref",
    "barcode",
    "item_ref",
    "price_kobo",
    "effective_at",
    "updates",
    "days",
];

/// Refuses a payload with a field that is not on the list, before any network call.
pub fn check_outbound(payload: &Value) -> Result<(), PartnerError> {
    match payload {
        Value::Object(map) => {
            for (k, v) in map {
                if !ALLOWED_OUTBOUND_FIELDS.contains(&k.as_str()) {
                    return Err(PartnerError::Blocked);
                }
                check_outbound(v)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(check_outbound),
        _ => Ok(()),
    }
}

#[async_trait]
pub trait PartnerApi: Send + Sync {
    /// `false` for [`NoPartner`].
    fn configured(&self) -> bool;
    async fn get(&self, path: &str) -> Result<Value, PartnerError>;
    /// `idempotency_key` is sent as `Idempotency-Key` so the partner can ignore a repeat.
    async fn post(
        &self,
        path: &str,
        idempotency_key: &str,
        body: &Value,
    ) -> Result<Value, PartnerError>;
}

/// Answers "not configured" without a network call.
pub struct NoPartner;

#[async_trait]
impl PartnerApi for NoPartner {
    fn configured(&self) -> bool {
        false
    }
    async fn get(&self, _path: &str) -> Result<Value, PartnerError> {
        Err(PartnerError::NotConfigured)
    }
    async fn post(&self, _p: &str, _k: &str, _b: &Value) -> Result<Value, PartnerError> {
        Err(PartnerError::NotConfigured)
    }
}

pub struct HttpPartner {
    base: Url,
    client: reqwest::Client,
    key: HeaderValue,
}

impl HttpPartner {
    pub fn new(cfg: PartnerConfig) -> anyhow::Result<Self> {
        Self::with_timeouts(cfg, CONNECT_TIMEOUT, REQUEST_TIMEOUT)
    }

    /// [`new`](Self::new) with other timeouts (tests make them short).
    pub fn with_timeouts(
        cfg: PartnerConfig,
        connect: Duration,
        total: Duration,
    ) -> anyhow::Result<Self> {
        let base = Url::parse(&format!("{}/", cfg.base_url.trim_end_matches('/')))
            .map_err(|_| anyhow!("PARTNER_API_BASE_URL is not a valid URL"))?;
        let mut key = HeaderValue::from_str(&format!("Bearer {}", cfg.api_key.expose()))
            .map_err(|_| anyhow!("PARTNER_API_KEY contains invalid characters"))?;
        key.set_sensitive(true);
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(connect)
            .timeout(total)
            .redirect(Policy::none())
            .https_only(!cfg.insecure_http_for_tests)
            .build()
            .map_err(|e| anyhow!(e.without_url()))?;
        Ok(Self { base, client, key })
    }

    fn url(&self, path: &str) -> Result<Url, PartnerError> {
        // A path never starts a new origin or climbs out of the base.
        if path.contains("..") || path.contains("//") || path.contains(char::is_whitespace) {
            return Err(PartnerError::Blocked);
        }
        self.base
            .join(path.trim_start_matches('/'))
            .map_err(|_| PartnerError::Blocked)
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        idempotency_key: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value, PartnerError> {
        let mut req = self
            .client
            .request(method, self.url(path)?)
            .header("authorization", self.key.clone());
        if let Some(k) = idempotency_key {
            req = req.header("idempotency-key", k);
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        let mut res = req.send().await.map_err(|e| {
            tracing::warn!(timeout = e.is_timeout(), "partner call failed");
            PartnerError::Unavailable
        })?;
        let status = res.status();
        if status.is_server_error() || status.as_u16() == 429 {
            return Err(PartnerError::Unavailable);
        }
        if !status.is_success() {
            return Err(PartnerError::Rejected);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(|_| PartnerError::Unavailable)? {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BODY {
                return Err(PartnerError::BadResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| PartnerError::BadResponse)
    }
}

#[async_trait]
impl PartnerApi for HttpPartner {
    fn configured(&self) -> bool {
        true
    }

    /// One retry, for a transient failure only: a `GET` can be repeated safely.
    async fn get(&self, path: &str) -> Result<Value, PartnerError> {
        match self.send(Method::GET, path, None, None).await {
            Err(PartnerError::Unavailable) => self.send(Method::GET, path, None, None).await,
            other => other,
        }
    }

    /// Sent once: a repeat could be a second charge or a second price change.
    async fn post(
        &self,
        path: &str,
        idempotency_key: &str,
        body: &Value,
    ) -> Result<Value, PartnerError> {
        check_outbound(body)?;
        self.send(Method::POST, path, Some(idempotency_key), Some(body))
            .await
    }
}
