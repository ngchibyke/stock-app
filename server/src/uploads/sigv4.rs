//! AWS Signature Version 4 for S3, written by hand with `hmac` and `sha2` so no
//! cloud SDK is pulled in. Two forms: a signed URL (the query string carries the
//! signature, for the phone's upload and for private downloads) and a signed
//! request (an `Authorization` header, for the server's own calls).
//!
//! Checked against the examples in AWS's documentation in `tests/s3_presign.rs`.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, UtcOffset, macros::format_description};

use crate::payments::mono::Secret;

type HmacSha256 = Hmac<Sha256>;

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const SERVICE: &str = "s3";
/// A signed URL does not commit to the body: the signed headers do the work.
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts a key of any length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Percent-encodes everything but `A-Za-z0-9-_.~`. A path keeps its slashes.
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The headers as the canonical request wants them (lower-case names, sorted,
/// trimmed values) and the list of names that were signed.
fn canonical_headers(headers: &[(&str, &str)]) -> (String, String) {
    let mut h: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| {
            let v = v.split_whitespace().collect::<Vec<_>>().join(" ");
            (k.to_lowercase(), v)
        })
        .collect();
    h.sort();
    let block = h
        .iter()
        .map(|(k, v)| format!("{k}:{v}\n"))
        .collect::<String>();
    let names = h
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    (block, names)
}

pub struct Signer {
    access_key_id: String,
    secret: Secret,
    region: String,
}

impl Signer {
    pub fn new(access_key_id: &str, secret: Secret, region: &str) -> Self {
        Self {
            access_key_id: access_key_id.to_string(),
            secret,
            region: region.to_string(),
        }
    }

    fn dates(now: OffsetDateTime) -> (String, String) {
        let now = now.to_offset(UtcOffset::UTC);
        (
            now.format(format_description!("[year][month][day]"))
                .expect("format date"),
            now.format(format_description!(
                "[year][month][day]T[hour][minute][second]Z"
            ))
            .expect("format timestamp"),
        )
    }

    fn scope(&self, date: &str) -> String {
        format!("{date}/{}/{SERVICE}/aws4_request", self.region)
    }

    fn signature(&self, date: &str, amz_date: &str, canonical_request: &str) -> String {
        let k_date = hmac(format!("AWS4{}", self.secret.expose()).as_bytes(), date);
        let k_region = hmac(&k_date, &self.region);
        let k_service = hmac(&k_region, SERVICE);
        let k_signing = hmac(&k_service, "aws4_request");
        let to_sign = format!(
            "{ALGORITHM}\n{amz_date}\n{}\n{}",
            self.scope(date),
            sha256_hex(canonical_request.as_bytes())
        );
        hex(&hmac(&k_signing, &to_sign))
    }

    /// A signed URL's query string. `path` is the unencoded path (`/bucket/key`);
    /// `headers` are signed in addition to `host` and must be sent by whoever
    /// uses the URL.
    pub fn presign(
        &self,
        method: &str,
        host: &str,
        path: &str,
        headers: &[(&str, &str)],
        expires_secs: u32,
        now: OffsetDateTime,
    ) -> String {
        let (date, amz_date) = Self::dates(now);
        let mut all = vec![("host", host)];
        all.extend_from_slice(headers);
        let (block, names) = canonical_headers(&all);
        let credential = format!("{}/{}", self.access_key_id, self.scope(&date));
        let query = format!(
            "X-Amz-Algorithm={ALGORITHM}&X-Amz-Credential={}&X-Amz-Date={amz_date}\
             &X-Amz-Expires={expires_secs}&X-Amz-SignedHeaders={}",
            uri_encode(&credential, false),
            uri_encode(&names, false)
        );
        let canonical = format!(
            "{method}\n{}\n{query}\n{block}\n{names}\n{UNSIGNED_PAYLOAD}",
            uri_encode(path, true)
        );
        let signature = self.signature(&date, &amz_date, &canonical);
        format!("{query}&X-Amz-Signature={signature}")
    }

    /// The `Authorization` header for a direct request. `headers` are every
    /// header to sign besides `host`, and must include `x-amz-date` (equal to
    /// `now`) and `x-amz-content-sha256` (equal to `payload_sha256`).
    #[allow(clippy::too_many_arguments)]
    pub fn authorize(
        &self,
        method: &str,
        host: &str,
        path: &str,
        query: &str,
        headers: &[(&str, &str)],
        payload_sha256: &str,
        now: OffsetDateTime,
    ) -> String {
        let (date, amz_date) = Self::dates(now);
        let mut all = vec![("host", host)];
        all.extend_from_slice(headers);
        let (block, names) = canonical_headers(&all);
        let canonical = format!(
            "{method}\n{}\n{query}\n{block}\n{names}\n{payload_sha256}",
            uri_encode(path, true)
        );
        let signature = self.signature(&date, &amz_date, &canonical);
        format!(
            "{ALGORITHM} Credential={}/{},SignedHeaders={names},Signature={signature}",
            self.access_key_id,
            self.scope(&date)
        )
    }

    /// The timestamp header value a direct request must carry for `now`.
    pub fn amz_date(now: OffsetDateTime) -> String {
        Self::dates(now).1
    }
}
