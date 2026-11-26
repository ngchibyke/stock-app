//! A scriptable [`IdentityProvider`] for development and tests. It refuses to be
//! registered in production (see `main.rs`). Its webhook is signed like a real one
//! (HMAC-SHA256 over the raw body, hex, in [`SIGNATURE_HEADER`]) so the signature
//! path is tested too.

use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use time::Date;
use uuid::Uuid;

use super::{
    FailReason, IdentityError, IdentityEvent, IdentityProvider, Nin, Outcome, StartedJob, Verified,
};

pub const FAKE_NAME: &str = "fake";
pub const SIGNATURE_HEADER: &str = "x-identity-signature";
pub const WEBHOOK_SECRET: &str = "fake-identity-webhook-secret";

#[derive(Default)]
pub struct FakeIdentity {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    starts: u32,
    fail_start: bool,
    last_nin: Option<String>,
    /// Results `fetch` answers, by provider reference.
    results: HashMap<String, Outcome>,
    fetches: u32,
}

impl FakeIdentity {
    pub fn new() -> Self {
        Self::default()
    }

    /// Calls to `start` so far.
    pub fn starts(&self) -> u32 {
        self.inner.lock().unwrap().starts
    }

    pub fn fetches(&self) -> u32 {
        self.inner.lock().unwrap().fetches
    }

    /// The NIN of the last `start` (tests check the provider received it).
    pub fn last_nin(&self) -> Option<String> {
        self.inner.lock().unwrap().last_nin.clone()
    }

    /// Makes `start` fail as if the provider were down.
    pub fn set_fail_start(&self, fail: bool) {
        self.inner.lock().unwrap().fail_start = fail;
    }

    /// What `fetch` answers for `provider_ref`.
    pub fn set_result(&self, provider_ref: &str, outcome: Outcome) {
        self.inner
            .lock()
            .unwrap()
            .results
            .insert(provider_ref.to_string(), outcome);
    }

    /// The signature header value and body of a webhook for `provider_ref`.
    pub fn webhook(provider_ref: &str, outcome: &Outcome) -> (String, Vec<u8>) {
        let body = match outcome {
            Outcome::Passed(v) => json!({
                "ref": provider_ref, "outcome": "passed",
                "full_name": v.full_name, "dob": v.date_of_birth.to_string(),
            }),
            Outcome::Failed(r) => json!({ "ref": provider_ref, "outcome": r.code() }),
            Outcome::Unsure => json!({ "ref": provider_ref, "outcome": "unsure" }),
        };
        let body = serde_json::to_vec(&body).unwrap();
        (sign(&body), body)
    }
}

fn sign(body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes()).unwrap();
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[async_trait]
impl IdentityProvider for FakeIdentity {
    fn name(&self) -> &'static str {
        FAKE_NAME
    }

    async fn start(&self, nin: &Nin, reference: Uuid) -> Result<StartedJob, IdentityError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.fail_start {
            return Err(IdentityError::Unavailable);
        }
        inner.starts += 1;
        inner.last_nin = Some(nin.expose().to_string());
        Ok(StartedJob {
            provider_ref: format!("fake-job-{reference}"),
            session_token: format!("fake-session-{reference}"),
        })
    }

    fn parse_webhook(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<IdentityEvent, IdentityError> {
        let given = headers
            .get(SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(hex_decode)
            .ok_or(IdentityError::InvalidSignature)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(WEBHOOK_SECRET.as_bytes())
            .map_err(|_| IdentityError::InvalidSignature)?;
        mac.update(body);
        mac.verify_slice(&given)
            .map_err(|_| IdentityError::InvalidSignature)?;
        let v: Value = serde_json::from_slice(body).map_err(|_| IdentityError::Malformed)?;
        let provider_ref = v["ref"]
            .as_str()
            .ok_or(IdentityError::Malformed)?
            .to_string();
        let outcome = match v["outcome"].as_str().ok_or(IdentityError::Malformed)? {
            "passed" => {
                let full_name = v["full_name"].as_str().ok_or(IdentityError::Malformed)?;
                let dob = v["dob"].as_str().ok_or(IdentityError::Malformed)?;
                let date_of_birth =
                    Date::parse(dob, &time::format_description::well_known::Iso8601::DATE)
                        .map_err(|_| IdentityError::Malformed)?;
                Outcome::Passed(Verified {
                    full_name: full_name.to_string(),
                    date_of_birth,
                })
            }
            "not_found" => Outcome::Failed(FailReason::NotFound),
            "face_mismatch" => Outcome::Failed(FailReason::FaceMismatch),
            "not_alive" => Outcome::Failed(FailReason::NotAlive),
            "unsure" => Outcome::Unsure,
            _ => return Err(IdentityError::Malformed),
        };
        Ok(IdentityEvent {
            provider_ref,
            outcome,
        })
    }

    async fn fetch(&self, provider_ref: &str) -> Result<Option<Outcome>, IdentityError> {
        let mut inner = self.inner.lock().unwrap();
        inner.fetches += 1;
        Ok(inner.results.get(provider_ref).cloned())
    }
}
