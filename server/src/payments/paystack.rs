//! Paystack adapter: Checkout for collection, Transfers for seller payouts, Refunds. Facts
//! are from Paystack's public documentation; shapes it does not document, and anything not
//! yet tried against a real account, are marked **unconfirmed**. See
//! `docs/superpowers/plans/2026-11-27-paystack-adapter.md` for the table of facts relied on.
//!
//! Rules this adapter keeps (the same as the Mono and Shipbubble adapters):
//! - **Data protection.** Bodies carry bank account numbers and may carry personal data:
//!   they are never logged or stored. Logs carry the operation, the HTTP status and a fixed
//!   error class. The secret key lives in [`Secret`] and its header is marked sensitive.
//!   What is sent is minimised: no customer name; an email only when the buyer has one,
//!   otherwise a synthetic address on a domain the platform owns; for payouts only the bank
//!   code and account number, under a fixed label.
//! - **No retries inside the adapter.** Every write is sent once. A failure after sending is
//!   [`ProviderError::Timeout`] (outcome unknown); only a connect failure, where nothing was
//!   sent, is [`ProviderError::Unavailable`]. A payout is looked up with
//!   [`find_payout`](PaymentProvider::find_payout) before it is ever created again, and a
//!   refund looks for its own earlier attempt before it is created.
//! - **Conservative mapping.** An unknown status is `Pending`, never a terminal state.
//! - **Amounts** are integer kobo, which is Paystack's own unit; nothing is converted.
//! - **HTTP client.** rustls, `https_only`, no redirects, a connect and a total timeout.

use std::{fmt, time::Duration};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use hmac::{Hmac, Mac};
use reqwest::{Method, Url, redirect::Policy};
use serde_json::{Value, json};
use sha2::Sha512;

use super::{
    InitiatePayment, InitiatedPayment, PaymentProvider, PaymentStatus, PayoutHandle, PayoutRequest,
    PayoutState, ProviderError, RefundHandle, RefundRequest, WebhookEvent, mono::Secret, secret_eq,
};

pub const PAYSTACK_NAME: &str = "paystack";
/// The one payment method this provider collects with.
pub const METHOD: &str = "paystack_checkout";
pub const DEFAULT_BASE_URL: &str = "https://api.paystack.co";
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest response body read; anything longer is treated like a failed read.
pub const MAX_RESPONSE_BODY: usize = 512 * 1024;

const SIGNATURE_HEADER: &str = "x-paystack-signature";
/// Hosts the hosted checkout page is served from (exact host or a subdomain). Documentation
/// shows `checkout.paystack.com`. **Unconfirmed** as a complete list.
const PAYMENT_URL_HOSTS: &[&str] = &["paystack.com", "paystack.co"];
/// What a payout recipient is called at Paystack: a fixed label, never the seller's name.
const RECIPIENT_LABEL: &str = "Marketplace seller";

#[derive(Clone)]
pub struct PaystackConfig {
    pub secret_key: Secret,
    /// `PAYSTACK_BASE_URL`, default [`DEFAULT_BASE_URL`].
    pub base_url: String,
    /// Live mode (`APP_ENV=production`): events marked as the other mode are ignored.
    pub live: bool,
    /// A domain the platform owns, for the synthetic payer email when a buyer has none
    /// (phone-only accounts have no email). Receipts go to that address.
    pub payer_email_domain: String,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    /// **Tests only** (a local mock speaks plain http). Config always sets `false`.
    pub insecure_http_for_tests: bool,
}

impl fmt::Debug for PaystackConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaystackConfig")
            .field("base_url", &self.base_url)
            .field("live", &self.live)
            .field("secret_key", &"[redacted]")
            .finish_non_exhaustive()
    }
}

/// Whether a call can have taken effect at Paystack when it fails after sending.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Call {
    /// GET: no side effects.
    Read,
    /// POST: a failure after sending leaves the outcome unknown.
    Write,
}

struct Answer {
    status: StatusCode,
    body: Option<Value>,
}

pub struct PaystackProvider {
    cfg: PaystackConfig,
    base: Url,
    client: reqwest::Client,
    key_header: HeaderValue,
}

impl fmt::Debug for PaystackProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PaystackProvider")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

impl PaystackProvider {
    pub fn new(cfg: PaystackConfig) -> anyhow::Result<Self> {
        let base = Url::parse(cfg.base_url.trim_end_matches('/'))
            .ok()
            .filter(|u| !u.cannot_be_a_base() && u.query().is_none())
            .context("PAYSTACK_BASE_URL is not a valid base URL")?;
        let mut key_header = HeaderValue::from_str(&format!("Bearer {}", cfg.secret_key.expose()))
            .map_err(|_| anyhow!("PAYSTACK_SECRET_KEY contains invalid characters"))?;
        key_header.set_sensitive(true);
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(cfg.connect_timeout)
            .timeout(cfg.timeout)
            .redirect(Policy::none())
            .https_only(!cfg.insecure_http_for_tests)
            .build()
            .map_err(|e| anyhow!(e.without_url()))
            .context("build Paystack HTTP client")?;
        Ok(Self {
            cfg,
            base,
            client,
            key_header,
        })
    }

    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        url
    }

    /// Sends one request (no retry) and reads the body. Never logs the body.
    async fn call(
        &self,
        op: &'static str,
        call: Call,
        method: Method,
        url: Url,
        body: Option<&Value>,
    ) -> Result<Answer, ProviderError> {
        let mut req = self
            .client
            .request(method, url)
            .header("authorization", self.key_header.clone())
            .header("accept", "application/json");
        if let Some(body) = body {
            req = req.json(body);
        }
        let mut res = req.send().await.map_err(|e| transport_error(op, call, e))?;
        let status = res.status();
        let oversize = || {
            tracing::warn!(
                provider = PAYSTACK_NAME,
                op,
                cause = "body_too_large",
                "paystack call failed"
            );
            match call {
                Call::Write => ProviderError::Timeout,
                Call::Read => ProviderError::Unavailable(anyhow!("paystack body too large")),
            }
        };
        if res
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BODY as u64)
        {
            return Err(oversize());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|e| transport_error(op, call, e))?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BODY {
                return Err(oversize());
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = serde_json::from_slice::<Value>(&bytes).ok();
        Ok(Answer { status, body })
    }

    /// A transfer by our reference. Paystack wants transfer references in lowercase.
    async fn transfer_by_reference(
        &self,
        op: &'static str,
        reference: &str,
    ) -> Result<Answer, ProviderError> {
        let url = self.url(&["transfer", "verify", &transfer_reference(reference)]);
        self.call(op, Call::Read, Method::GET, url, None).await
    }
}

/// Our payout reference (uppercase letters and digits) as Paystack wants it: lowercase,
/// 16 to 50 characters. Our references are 22 to 24 characters, so the length is fine.
fn transfer_reference(reference: &str) -> String {
    reference.to_ascii_lowercase()
}

/// A refusal decided here, before anything is sent.
fn rejected_locally(code: &str) -> ProviderError {
    ProviderError::Rejected {
        code: code.to_string(),
        message: String::new(),
    }
}

/// A 2xx body we cannot read: the outcome is unknown (trait contract).
fn unparseable(op: &'static str) -> ProviderError {
    tracing::warn!(
        provider = PAYSTACK_NAME,
        op,
        cause = "unparseable_body",
        "paystack call failed"
    );
    ProviderError::Timeout
}

/// Maps a reqwest failure. Logs only the sanitised class, never the error itself (it may
/// hold the URL, which carries references).
fn transport_error(op: &'static str, call: Call, e: reqwest::Error) -> ProviderError {
    let e = e.without_url();
    let (cause, err) = if e.is_connect() {
        (
            "connect",
            ProviderError::Unavailable(anyhow!("paystack connect failed")),
        )
    } else if e.is_timeout() {
        ("timeout", ProviderError::Timeout)
    } else if call == Call::Write {
        ("transport_after_send", ProviderError::Timeout)
    } else {
        (
            "transport",
            ProviderError::Unavailable(anyhow!("paystack request failed")),
        )
    };
    tracing::warn!(provider = PAYSTACK_NAME, op, cause, "paystack call failed");
    err
}

/// Maps a non-2xx answer: 429 is "not processed" (Unavailable); other 4xx is a refusal
/// (Rejected); 5xx on a read is Unavailable and on a write may have taken effect (Timeout).
fn status_error(op: &'static str, call: Call, answer: &Answer) -> ProviderError {
    let status = answer.status.as_u16();
    tracing::warn!(
        provider = PAYSTACK_NAME,
        op,
        status,
        cause = "http_status",
        "paystack call failed"
    );
    if answer.status == StatusCode::TOO_MANY_REQUESTS {
        ProviderError::Unavailable(anyhow!("paystack rate limited"))
    } else if answer.status.is_client_error() {
        ProviderError::Rejected {
            code: format!("http_{status}"),
            message: safe_message(answer.body.as_ref()),
        }
    } else if call == Call::Write {
        ProviderError::Timeout
    } else {
        ProviderError::Unavailable(anyhow!("paystack answered {status}"))
    }
}

/// Paystack's top-level `message`, kept for support only when it is short and holds no run
/// of 6 or more digits (account numbers, phone numbers).
fn safe_message(body: Option<&Value>) -> String {
    let Some(msg) = body.and_then(|b| str_at(b, "message")) else {
        return String::new();
    };
    let mut run = 0;
    let has_long_number = msg.chars().any(|c| {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        run >= 6
    });
    if has_long_number || msg.chars().count() > 160 {
        "[message withheld]".into()
    } else {
        msg.to_string()
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// An id that may be a JSON string or number.
fn id_at(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A kobo amount that may be a JSON integer or a numeric string; nothing fractional.
fn kobo_at(v: &Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// The `data` object of a 2xx answer, or an unreadable-body error. Paystack also answers
/// `status: false` with a 2xx now and then; that is a refusal.
fn data_of<'a>(op: &'static str, answer: &'a Answer) -> Result<&'a Value, ProviderError> {
    let body = answer.body.as_ref().ok_or_else(|| unparseable(op))?;
    if body.get("status").and_then(Value::as_bool) == Some(false) {
        return Err(rejected_locally("paystack_refused"));
    }
    body.get("data")
        .filter(|d| d.is_object() || d.is_array())
        .ok_or_else(|| unparseable(op))
}

fn mentions_not_found(body: Option<&Value>) -> bool {
    body.and_then(|v| str_at(v, "message"))
        .is_some_and(|m| m.to_ascii_lowercase().contains("not found"))
}

/// A refused create that may hide an earlier, successful attempt.
fn is_possible_duplicate(answer: &Answer) -> bool {
    if matches!(answer.status.as_u16(), 408 | 409 | 425) {
        return true;
    }
    answer.status.is_client_error()
        && answer
            .body
            .as_ref()
            .and_then(|b| str_at(b, "message"))
            .map(str::to_ascii_lowercase)
            .is_some_and(|m| m.contains("duplicate") || m.contains("already"))
}

fn valid_email(e: &str) -> bool {
    !e.contains(char::is_whitespace)
        && e.split_once('@')
            .is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !d.contains('@'))
}

#[async_trait]
impl PaymentProvider for PaystackProvider {
    fn name(&self) -> &'static str {
        PAYSTACK_NAME
    }

    fn methods(&self) -> &'static [&'static str] {
        &[METHOD]
    }

    fn payment_url_hosts(&self) -> &'static [&'static str] {
        PAYMENT_URL_HOSTS
    }

    async fn initiate(&self, req: InitiatePayment) -> Result<InitiatedPayment, ProviderError> {
        const OP: &str = "initiate";
        if req.amount_kobo <= 0 {
            return Err(rejected_locally("invalid_amount"));
        }
        // Paystack requires an email. Use the buyer's own only if they gave one; otherwise a
        // synthetic address on our domain. No name is sent.
        let email = match req.customer_email.as_deref().filter(|e| valid_email(e)) {
            Some(e) => e.to_string(),
            None => format!(
                "{}@{}",
                req.reference.to_ascii_lowercase(),
                self.cfg.payer_email_domain
            ),
        };
        let body = json!({
            "email": email,
            "amount": req.amount_kobo,
            "currency": "NGN",
            "reference": req.reference,
            "callback_url": req.redirect_url,
        });
        let url = self.url(&["transaction", "initialize"]);
        let answer = self
            .call(OP, Call::Write, Method::POST, url, Some(&body))
            .await?;
        if !answer.status.is_success() {
            return Err(status_error(OP, Call::Write, &answer));
        }
        let data = data_of(OP, &answer)?;
        let payment_url = str_at(data, "authorization_url").ok_or_else(|| unparseable(OP))?;
        let id = str_at(data, "access_code")
            .map(str::to_string)
            .or_else(|| str_at(data, "reference").map(str::to_string))
            .unwrap_or_else(|| req.reference.clone());
        Ok(InitiatedPayment {
            provider_payment_id: id,
            payment_url: payment_url.to_string(),
        })
    }

    async fn verify_payment(&self, reference: &str) -> Result<PaymentStatus, ProviderError> {
        const OP: &str = "verify_payment";
        let url = self.url(&["transaction", "verify", reference]);
        let answer = self.call(OP, Call::Read, Method::GET, url, None).await?;
        // A reference Paystack has never seen is a payment that has not started.
        if answer.status == StatusCode::NOT_FOUND
            || (answer.status.is_client_error() && mentions_not_found(answer.body.as_ref()))
        {
            return Ok(PaymentStatus::Pending);
        }
        if !answer.status.is_success() {
            return Err(status_error(OP, Call::Read, &answer));
        }
        let data = data_of(OP, &answer)?;
        let status = str_at(data, "status")
            .unwrap_or_default()
            .to_ascii_lowercase();
        Ok(match status.as_str() {
            "success" => PaymentStatus::Successful {
                amount_kobo: kobo_at(data, "amount").ok_or_else(|| unparseable(OP))?,
                fee_kobo: kobo_at(data, "fees").unwrap_or(0),
                currency: str_at(data, "currency").unwrap_or("NGN").to_string(),
                // Paystack settles to our balance: nothing is held at the provider for us.
                held: false,
                reference: str_at(data, "reference").unwrap_or(reference).to_string(),
            },
            "failed" | "reversed" => PaymentStatus::Failed,
            // abandoned, ongoing, pending, processing, queued, and anything unknown.
            _ => PaymentStatus::Pending,
        })
    }

    async fn create_payout(&self, req: PayoutRequest) -> Result<PayoutHandle, ProviderError> {
        const OP: &str = "create_payout";
        if req.amount_kobo <= 0 {
            return Err(rejected_locally("invalid_amount"));
        }
        // Step 1: the recipient. No money has moved, so any failure here is safe to retry.
        let body = json!({
            "type": "nuban",
            "name": RECIPIENT_LABEL,
            "account_number": req.account_number,
            "bank_code": req.bank_code,
            "currency": "NGN",
        });
        let url = self.url(&["transferrecipient"]);
        let answer = self
            .call(
                "create_recipient",
                Call::Write,
                Method::POST,
                url,
                Some(&body),
            )
            .await
            .map_err(|e| match e {
                ProviderError::Timeout => ProviderError::Unavailable(anyhow!("recipient step")),
                other => other,
            })?;
        if !answer.status.is_success() {
            return Err(
                match status_error("create_recipient", Call::Write, &answer) {
                    ProviderError::Timeout => ProviderError::Unavailable(anyhow!("recipient step")),
                    other => other,
                },
            );
        }
        let recipient = data_of("create_recipient", &answer)
            .ok()
            .and_then(|d| str_at(d, "recipient_code"))
            .ok_or_else(|| ProviderError::Unavailable(anyhow!("recipient unreadable")))?
            .to_string();

        // Step 2: the transfer, sent once. Its reference is ours, so a repeat is findable.
        let reference = transfer_reference(&req.reference);
        let body = json!({
            "source": "balance",
            "amount": req.amount_kobo,
            "recipient": recipient,
            "reference": reference,
            "reason": req.narration,
        });
        let url = self.url(&["transfer"]);
        let answer = self
            .call(OP, Call::Write, Method::POST, url, Some(&body))
            .await?;
        if !answer.status.is_success() {
            // A refused create that may hide an earlier success: look before giving up.
            if is_possible_duplicate(&answer)
                && let Some(handle) = self.find_payout(&req.reference).await?
            {
                return Ok(handle);
            }
            return Err(status_error(OP, Call::Write, &answer));
        }
        let data = data_of(OP, &answer)?;
        if str_at(data, "status").is_some_and(|s| s.eq_ignore_ascii_case("failed")) {
            return Err(rejected_locally("transfer_failed"));
        }
        let id = str_at(data, "transfer_code")
            .map(str::to_string)
            .or_else(|| id_at(data, "id"))
            .ok_or_else(|| unparseable(OP))?;
        Ok(PayoutHandle {
            provider_payout_id: id,
        })
    }

    async fn payout_status(
        &self,
        reference: &str,
        _provider_payout_id: Option<&str>,
    ) -> Result<PayoutState, ProviderError> {
        const OP: &str = "payout_status";
        let answer = self.transfer_by_reference(OP, reference).await?;
        if !answer.status.is_success() {
            return Err(status_error(OP, Call::Read, &answer));
        }
        let data = data_of(OP, &answer)?;
        let status = str_at(data, "status")
            .unwrap_or_default()
            .to_ascii_lowercase();
        Ok(match status.as_str() {
            "success" => {
                // Unconfirmed: where Paystack reports the transfer fee. Absent means 0.
                let fee_kobo = ["fee_charged", "fees_charged", "fee"]
                    .iter()
                    .find_map(|k| kobo_at(data, k))
                    .unwrap_or(0);
                PayoutState::Successful {
                    fee_kobo,
                    amount_kobo: kobo_at(data, "amount"),
                }
            }
            // Fixed text only: Paystack's own message can echo the account.
            "failed" => PayoutState::Failed {
                reason: "paystack_transfer_failed".into(),
            },
            "reversed" => PayoutState::Reversed {
                reason: "paystack_transfer_reversed".into(),
            },
            // pending, otp, received, processing, and anything unknown.
            _ => PayoutState::Pending,
        })
    }

    async fn find_payout(&self, reference: &str) -> Result<Option<PayoutHandle>, ProviderError> {
        const OP: &str = "find_payout";
        let answer = self.transfer_by_reference(OP, reference).await?;
        if answer.status == StatusCode::NOT_FOUND
            || (answer.status.is_client_error() && mentions_not_found(answer.body.as_ref()))
        {
            return Ok(None);
        }
        if !answer.status.is_success() {
            return Err(status_error(OP, Call::Read, &answer));
        }
        let data = data_of(OP, &answer)?;
        let id = str_at(data, "transfer_code")
            .map(str::to_string)
            .or_else(|| id_at(data, "id"))
            .ok_or_else(|| unparseable(OP))?;
        Ok(Some(PayoutHandle {
            provider_payout_id: id,
        }))
    }

    /// Completes a transfer that Paystack holds at the OTP step. Turn the OTP requirement
    /// off for API transfers in the Paystack dashboard to avoid needing this at all.
    async fn finalize_payout(&self, reference: &str, otp: &str) -> Result<(), ProviderError> {
        const OP: &str = "finalize_payout";
        let found = self.transfer_by_reference(OP, reference).await?;
        if !found.status.is_success() {
            return Err(status_error(OP, Call::Read, &found));
        }
        let data = data_of(OP, &found)?;
        let code = str_at(data, "transfer_code").ok_or_else(|| unparseable(OP))?;
        let body = json!({ "transfer_code": code, "otp": otp });
        let url = self.url(&["transfer", "finalize_transfer"]);
        let answer = self
            .call(OP, Call::Write, Method::POST, url, Some(&body))
            .await?;
        if !answer.status.is_success() {
            return Err(status_error(OP, Call::Write, &answer));
        }
        data_of(OP, &answer).map(|_| ())
    }

    async fn refund(&self, req: RefundRequest) -> Result<RefundHandle, ProviderError> {
        const OP: &str = "refund";
        if req.amount_kobo <= 0 {
            return Err(rejected_locally("invalid_amount"));
        }
        // Paystack has no idempotency key for refunds, so look for our own earlier attempt
        // first (its `merchant_note` is our refund reference) and reuse it.
        let mut url = self.url(&["refund"]);
        url.query_pairs_mut()
            .append_pair("reference", &req.payment_reference)
            .append_pair("perPage", "50");
        let listed = self
            .call("refund_list", Call::Read, Method::GET, url, None)
            .await?;
        if listed.status.is_success()
            && let Some(rows) = listed
                .body
                .as_ref()
                .and_then(|b| b.get("data"))
                .and_then(Value::as_array)
            && let Some(mine) = rows.iter().find(|r| {
                str_at(r, "merchant_note") == Some(req.reference.as_str())
                    && !str_at(r, "status").is_some_and(|s| s.eq_ignore_ascii_case("failed"))
            })
            && let Some(id) = id_at(mine, "id")
        {
            return Ok(RefundHandle {
                provider_refund_id: id,
            });
        }
        if !listed.status.is_success() && !listed.status.is_client_error() {
            // We could not look, and a blind create might double-refund.
            return Err(status_error("refund_list", Call::Read, &listed));
        }
        let body = json!({
            "transaction": req.payment_reference,
            "amount": req.amount_kobo,
            "currency": "NGN",
            "merchant_note": req.reference,
            "customer_note": "Marketplace order refund",
        });
        let url = self.url(&["refund"]);
        let answer = self
            .call(OP, Call::Write, Method::POST, url, Some(&body))
            .await?;
        if !answer.status.is_success() {
            return Err(status_error(OP, Call::Write, &answer));
        }
        let data = data_of(OP, &answer)?;
        if str_at(data, "status").is_some_and(|s| s.eq_ignore_ascii_case("failed")) {
            return Err(rejected_locally("refund_failed"));
        }
        let id = id_at(data, "id")
            .or_else(|| str_at(data, "refund_reference").map(str::to_string))
            .ok_or_else(|| unparseable(OP))?;
        Ok(RefundHandle {
            provider_refund_id: id,
        })
    }

    async fn resolve_account(
        &self,
        bank_code: &str,
        account_number: &str,
    ) -> Result<String, ProviderError> {
        const OP: &str = "resolve_account";
        let mut url = self.url(&["bank", "resolve"]);
        url.query_pairs_mut()
            .append_pair("account_number", account_number)
            .append_pair("bank_code", bank_code);
        let answer = self.call(OP, Call::Read, Method::GET, url, None).await?;
        if !answer.status.is_success() {
            return Err(status_error(OP, Call::Read, &answer));
        }
        let data = data_of(OP, &answer)?;
        str_at(data, "account_name")
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .ok_or_else(|| ProviderError::Inconclusive)
    }

    fn verify_webhook(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<WebhookEvent, ProviderError> {
        // The signature is an HMAC-SHA512 of the raw body under the secret key, in hex.
        let given = headers
            .get(SIGNATURE_HEADER)
            .map(HeaderValue::as_bytes)
            .unwrap_or_default();
        let mut mac = Hmac::<Sha512>::new_from_slice(self.cfg.secret_key.expose().as_bytes())
            .map_err(|_| ProviderError::InvalidSignature)?;
        mac.update(body);
        let expected: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if given.is_empty() || !secret_eq(&given.to_ascii_lowercase(), expected.as_bytes()) {
            return Err(ProviderError::InvalidSignature);
        }
        let malformed = || rejected_locally("malformed_webhook");
        let v: Value = serde_json::from_slice(body).map_err(|_| malformed())?;
        let event = str_at(&v, "event").ok_or_else(malformed)?;
        let data = v.get("data").unwrap_or(&Value::Null);
        let reference = str_at(data, "reference").filter(|r| !r.trim().is_empty());
        let suffix = reference
            .map(|r| r.to_ascii_uppercase())
            .or_else(|| id_at(data, "id"))
            .unwrap_or_else(|| "none".into());
        let event_id = format!("{event}:{suffix}");
        // Events from the other mode are not ours (a test charge must not settle a live order).
        let mode = str_at(data, "domain");
        if mode.is_some_and(|d| (d == "live") != self.cfg.live) {
            tracing::info!(
                provider = PAYSTACK_NAME,
                "paystack webhook from the other mode ignored"
            );
            return Ok(WebhookEvent::Ignored { event_id });
        }
        Ok(match (event, reference) {
            // Each only triggers a verify: the verify answer decides what happens.
            ("charge.success", Some(r)) => WebhookEvent::Payment {
                event_id,
                reference: r.to_string(),
            },
            ("transfer.success" | "transfer.failed" | "transfer.reversed", Some(r)) => {
                // We sent a lowercase reference; ours are uppercase.
                WebhookEvent::Payout {
                    event_id,
                    reference: r.to_ascii_uppercase(),
                }
            }
            _ => WebhookEvent::Ignored { event_id },
        })
    }
}
