//! Where photo bytes live: the [`ObjectStore`] trait, an in-memory store for
//! tests and an S3-compatible one (S3, Cloudflare R2, MinIO, Backblaze) that signs
//! its own requests with AWS Signature V4.

use std::{collections::HashMap, sync::Mutex, time::Duration};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, macros::format_description};

use crate::payments::mono::Secret;

#[derive(Debug, thiserror::Error)]
#[error("object store failed")]
pub struct StoreError;

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<(), StoreError>;
}

/// **Test and development only.** Keeps objects in memory.
#[derive(Default)]
pub struct MemoryStore {
    objects: Mutex<HashMap<String, (Vec<u8>, String)>>,
    fail: Mutex<bool>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &str) -> Option<(Vec<u8>, String)> {
        self.objects.lock().unwrap().get(key).cloned()
    }

    pub fn len(&self) -> usize {
        self.objects.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Makes every `put` fail until called with `false`.
    pub fn fail(&self, on: bool) {
        *self.fail.lock().unwrap() = on;
    }
}

#[async_trait]
impl ObjectStore for MemoryStore {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<(), StoreError> {
        if *self.fail.lock().unwrap() {
            return Err(StoreError);
        }
        self.objects
            .lock()
            .unwrap()
            .insert(key.to_string(), (bytes, content_type.to_string()));
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct S3Config {
    /// `https://<account>.r2.cloudflarestorage.com`, `https://s3.eu-west-1.amazonaws.com`, ...
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: Secret,
    /// Where the app reads photos from: the public URL of the bucket or its CDN.
    pub photo_base_url: String,
    /// **Tests only** (a local mock speaks plain http). `config.rs` always sets `false`.
    pub insecure_http_for_tests: bool,
}

pub struct S3Store {
    cfg: S3Config,
    client: reqwest::Client,
}

impl S3Store {
    pub fn new(cfg: S3Config) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .redirect(Policy::none())
            .https_only(!cfg.insecure_http_for_tests)
            .build()
            .map_err(|e| anyhow!(e.without_url()))
            .context("build object store HTTP client")?;
        Ok(Self { cfg, client })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// One request to sign. `headers` are the ones to sign, names lowercase, values trimmed.
pub struct SignInput<'a> {
    pub method: &'a str,
    /// Already URI-encoded, starting with `/`.
    pub canonical_uri: &'a str,
    pub canonical_query: &'a str,
    pub headers: &'a [(&'a str, String)],
    pub payload_sha256: &'a str,
    /// `20130524T000000Z`.
    pub amz_date: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
}

/// The `Authorization` header value for [`SignInput`] (AWS Signature Version 4).
pub fn authorization(i: &SignInput) -> String {
    let date = &i.amz_date[..8];
    let mut headers: Vec<&(&str, String)> = i.headers.iter().collect();
    headers.sort_by(|a, b| a.0.cmp(b.0));
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers
        .iter()
        .map(|(k, _)| *k)
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        i.method,
        i.canonical_uri,
        i.canonical_query,
        canonical_headers,
        signed_headers,
        i.payload_sha256
    );
    let scope = format!("{date}/{}/{}/aws4_request", i.region, i.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        i.amz_date,
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac(format!("AWS4{}", i.secret_access_key).as_bytes(), date);
    let k_region = hmac(&k_date, i.region);
    let k_service = hmac(&k_region, i.service);
    let k_signing = hmac(&k_service, "aws4_request");
    let signature = hex(&hmac(&k_signing, &string_to_sign));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        i.access_key_id
    )
}

fn amz_date(now: OffsetDateTime) -> String {
    now.format(&format_description!(
        "[year][month][day]T[hour][minute][second]Z"
    ))
    .expect("a fixed format")
}

#[async_trait]
impl ObjectStore for S3Store {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<(), StoreError> {
        // Keys are `photos/<uuid>.<ext>`: nothing in them needs URI-encoding.
        let uri = format!("/{}/{}", self.cfg.bucket, key);
        let url = format!("{}{uri}", self.cfg.endpoint.trim_end_matches('/'));
        let host = reqwest::Url::parse(&url)
            .ok()
            .and_then(|u| {
                u.host_str()
                    .map(|h| u.port().map_or(h.to_string(), |p| format!("{h}:{p}")))
            })
            .ok_or(StoreError)?;
        let payload = sha256_hex(&bytes);
        let now = amz_date(OffsetDateTime::now_utc());
        let headers = [
            ("content-type", content_type.to_string()),
            ("host", host),
            ("x-amz-content-sha256", payload.clone()),
            ("x-amz-date", now.clone()),
        ];
        let auth = authorization(&SignInput {
            method: "PUT",
            canonical_uri: &uri,
            canonical_query: "",
            headers: &headers,
            payload_sha256: &payload,
            amz_date: &now,
            region: &self.cfg.region,
            service: "s3",
            access_key_id: &self.cfg.access_key_id,
            secret_access_key: self.cfg.secret_access_key.expose(),
        });
        let res = self
            .client
            .put(&url)
            .header("content-type", content_type)
            .header("x-amz-content-sha256", &payload)
            .header("x-amz-date", &now)
            .header("authorization", auth)
            .body(bytes)
            .send()
            .await
            .map_err(|e| {
                // Never log the URL or the error body: they carry the bucket and key.
                tracing::warn!(error = %e.without_url(), "object store put failed");
                StoreError
            })?;
        if res.status().is_success() {
            Ok(())
        } else {
            tracing::warn!(status = %res.status(), "object store refused a put");
            Err(StoreError)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS's own worked example "GET Object" (Signature Calculations for the
    /// Authorization Header, Signature Version 4): the signature below is the one
    /// AWS publishes for these exact inputs.
    #[test]
    fn matches_the_signature_aws_publishes_for_its_get_object_example() {
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let headers = [
            ("host", "examplebucket.s3.amazonaws.com".to_string()),
            ("range", "bytes=0-9".to_string()),
            ("x-amz-content-sha256", empty.to_string()),
            ("x-amz-date", "20130524T000000Z".to_string()),
        ];
        let auth = authorization(&SignInput {
            method: "GET",
            canonical_uri: "/test.txt",
            canonical_query: "",
            headers: &headers,
            payload_sha256: empty,
            amz_date: "20130524T000000Z",
            region: "us-east-1",
            service: "s3",
            access_key_id: "AKIAIOSFODNN7EXAMPLE",
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        });
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn formats_the_request_date() {
        let t = time::macros::datetime!(2013-05-24 00:00:00 UTC);
        assert_eq!(amz_date(t), "20130524T000000Z");
    }
}
