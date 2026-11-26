//! An S3-compatible object store over raw HTTP (AWS S3, Cloudflare R2, MinIO and
//! the like), signed with [`sigv4`](super::sigv4).
//!
//! Rules this adapter keeps, as the other adapters do:
//! - **No secrets in sight.** The secret key lives in a redacting `Secret`; no
//!   URL, key name, header or response text reaches an error or a log. A failure
//!   logs a fixed cause and the HTTP status.
//! - https only (http only in tests), no redirects, a short connect and total
//!   timeout, and every read is capped.
//! - **No retries here.** The caller decides.

use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use async_trait::async_trait;
use reqwest::{Method, Url, header::HeaderValue, redirect::Policy};
use time::OffsetDateTime;

use super::{
    sigv4::{Signer, sha256_hex, uri_encode},
    store::{Bucket, ObjectStore, Presigned, StoreError},
};
pub use crate::payments::mono::Secret;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest a signed URL may live (S3's own limit is seven days).
const MAX_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Debug, Clone)]
pub struct S3Config {
    /// The store's address, for example `https://s3.af-south-1.amazonaws.com`.
    pub endpoint: String,
    pub region: String,
    pub bucket_public: String,
    pub bucket_private: String,
    pub access_key_id: String,
    pub secret_access_key: Secret,
    /// Where public objects are served from (a CDN or the bucket's public address).
    pub public_base_url: String,
    /// `https://endpoint/bucket/key` when true, `https://bucket.endpoint/key` when false.
    pub path_style: bool,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    /// **Tests only** (a local mock server speaks plain http). `config.rs` always
    /// sets `false`.
    pub insecure_http_for_tests: bool,
}

pub struct S3Store {
    cfg: S3Config,
    signer: Signer,
    endpoint: Url,
    client: reqwest::Client,
}

impl S3Store {
    pub fn new(cfg: S3Config) -> anyhow::Result<Self> {
        let endpoint = Url::parse(&cfg.endpoint).context("S3_ENDPOINT is not a valid URL")?;
        if endpoint.host_str().is_none() {
            bail!("S3_ENDPOINT has no host");
        }
        if !cfg.insecure_http_for_tests && endpoint.scheme() != "https" {
            bail!("S3_ENDPOINT must be an https URL");
        }
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(cfg.connect_timeout)
            .timeout(cfg.timeout)
            .redirect(Policy::none())
            .https_only(!cfg.insecure_http_for_tests)
            .build()
            .map_err(|e| anyhow!(e.without_url()))
            .context("build the object store HTTP client")?;
        let signer = Signer::new(
            &cfg.access_key_id,
            cfg.secret_access_key.clone(),
            &cfg.region,
        );
        Ok(Self {
            cfg,
            signer,
            endpoint,
            client,
        })
    }

    fn bucket_name(&self, bucket: Bucket) -> &str {
        match bucket {
            Bucket::Public => &self.cfg.bucket_public,
            Bucket::Private => &self.cfg.bucket_private,
        }
    }

    /// The `Host` value, the unencoded path and the `scheme://host` base for an object.
    fn locate(&self, bucket: Bucket, key: &str) -> Result<(String, String, String), StoreError> {
        // Keys are made here (`u/{purpose}/{uuid}.jpg`, `raw/{uuid}`): refuse anything else.
        let plain = !key.is_empty()
            && key.len() <= 512
            && !key.starts_with('/')
            && !key.contains("..")
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'));
        if !plain {
            return Err(StoreError::Unavailable);
        }
        let host = self.endpoint.host_str().ok_or(StoreError::Unavailable)?;
        let authority = match self.endpoint.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        };
        let bucket = self.bucket_name(bucket);
        let (host, path) = if self.cfg.path_style {
            (authority, format!("/{bucket}/{key}"))
        } else {
            (format!("{bucket}.{authority}"), format!("/{key}"))
        };
        let base = format!("{}://{host}", self.endpoint.scheme());
        Ok((host, path, base))
    }

    fn ttl_secs(ttl: Duration) -> Result<u32, StoreError> {
        if ttl.is_zero() || ttl > MAX_TTL {
            return Err(StoreError::Unavailable);
        }
        Ok(ttl.as_secs() as u32)
    }

    /// [`ObjectStore::presign_put`] at a given moment (tests fix the clock).
    pub fn presign_put_at(
        &self,
        bucket: Bucket,
        key: &str,
        content_type: &str,
        content_length: u64,
        ttl: Duration,
        now: OffsetDateTime,
    ) -> Result<Presigned, StoreError> {
        let (host, path, base) = self.locate(bucket, key)?;
        let length = content_length.to_string();
        let query = self.signer.presign(
            "PUT",
            &host,
            &path,
            &[("content-length", &length), ("content-type", content_type)],
            Self::ttl_secs(ttl)?,
            now,
        );
        Ok(Presigned {
            url: format!("{base}{}?{query}", uri_encode(&path, true)),
            headers: vec![
                ("content-type".into(), content_type.into()),
                ("content-length".into(), length),
            ],
        })
    }

    /// [`ObjectStore::presign_get`] at a given moment.
    pub fn presign_get_at(
        &self,
        bucket: Bucket,
        key: &str,
        ttl: Duration,
        now: OffsetDateTime,
    ) -> Result<String, StoreError> {
        let (host, path, base) = self.locate(bucket, key)?;
        let query = self
            .signer
            .presign("GET", &host, &path, &[], Self::ttl_secs(ttl)?, now);
        Ok(format!("{base}{}?{query}", uri_encode(&path, true)))
    }

    /// One signed request. `body` is the bytes and content type of a `PUT`.
    async fn send(
        &self,
        method: Method,
        bucket: Bucket,
        key: &str,
        body: Option<(Vec<u8>, &str)>,
        cache_control: Option<&str>,
    ) -> Result<reqwest::Response, StoreError> {
        let (host, path, base) = self.locate(bucket, key)?;
        let now = OffsetDateTime::now_utc();
        let amz_date = Signer::amz_date(now);
        let payload = sha256_hex(body.as_ref().map_or(&[][..], |(b, _)| b));
        let mut signed = vec![
            ("x-amz-content-sha256", payload.as_str()),
            ("x-amz-date", amz_date.as_str()),
        ];
        if let Some((_, content_type)) = &body {
            signed.push(("content-type", content_type));
        }
        if let Some(cache_control) = cache_control {
            signed.push(("cache-control", cache_control));
        }
        let authorization =
            self.signer
                .authorize(method.as_str(), &host, &path, "", &signed, &payload, now);
        let mut auth = HeaderValue::from_str(&authorization).map_err(|_| failed("header", None))?;
        auth.set_sensitive(true);
        let mut request = self
            .client
            .request(method, format!("{base}{}", uri_encode(&path, true)))
            .header("authorization", auth)
            .header("x-amz-date", amz_date)
            .header("x-amz-content-sha256", payload);
        if let Some((bytes, content_type)) = body {
            request = request.header("content-type", content_type).body(bytes);
        }
        if let Some(cache_control) = cache_control {
            request = request.header("cache-control", cache_control);
        }
        request.send().await.map_err(|_| failed("transport", None))
    }
}

#[async_trait]
impl ObjectStore for S3Store {
    fn public_url(&self, key: &str) -> Option<String> {
        Some(format!(
            "{}/{key}",
            self.cfg.public_base_url.trim_end_matches('/')
        ))
    }

    fn presign_put(
        &self,
        bucket: Bucket,
        key: &str,
        content_type: &str,
        content_length: u64,
        ttl: Duration,
    ) -> Result<Presigned, StoreError> {
        self.presign_put_at(
            bucket,
            key,
            content_type,
            content_length,
            ttl,
            OffsetDateTime::now_utc(),
        )
    }

    fn presign_get(&self, bucket: Bucket, key: &str, ttl: Duration) -> Result<String, StoreError> {
        self.presign_get_at(bucket, key, ttl, OffsetDateTime::now_utc())
    }

    async fn get(
        &self,
        bucket: Bucket,
        key: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, StoreError> {
        let mut res = self.send(Method::GET, bucket, key, None, None).await?;
        let status = res.status();
        if status.as_u16() == 404 {
            return Err(StoreError::NotFound);
        }
        if !status.is_success() {
            return Err(failed("status", Some(status.as_u16())));
        }
        if res.content_length().is_some_and(|n| n > max_bytes as u64) {
            return Err(StoreError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|_| failed("read", Some(status.as_u16())))?
        {
            if body.len() + chunk.len() > max_bytes {
                return Err(StoreError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn put(
        &self,
        bucket: Bucket,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StoreError> {
        let res = self
            .send(Method::PUT, bucket, key, Some((bytes, content_type)), None)
            .await?;
        let status = res.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(failed("status", Some(status.as_u16())))
        }
    }

    async fn put_cached(
        &self,
        bucket: Bucket,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
        cache_control: &str,
    ) -> Result<(), StoreError> {
        let res = self
            .send(
                Method::PUT,
                bucket,
                key,
                Some((bytes, content_type)),
                Some(cache_control),
            )
            .await?;
        let status = res.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(failed("status", Some(status.as_u16())))
        }
    }

    async fn delete(&self, bucket: Bucket, key: &str) -> Result<(), StoreError> {
        let res = self.send(Method::DELETE, bucket, key, None, None).await?;
        let status = res.status();
        if status.is_success() || status.as_u16() == 404 {
            Ok(())
        } else {
            Err(failed("status", Some(status.as_u16())))
        }
    }
}

/// Logs a failure with a fixed cause, never a body, URL or key, and gives the error.
fn failed(cause: &'static str, status: Option<u16>) -> StoreError {
    tracing::warn!(cause, status, "object store failed");
    StoreError::Unavailable
}
