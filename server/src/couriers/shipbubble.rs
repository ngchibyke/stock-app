//! Shipbubble adapter (`docs.shipbubble.com`, research of 2026-10-31). Shapes
//! the docs do not show are marked **assumed**.
//!
//! Rules this adapter keeps:
//! - **Data protection.** Request and response bodies carry names, phones and
//!   addresses: they are never logged or stored. Logs carry only the
//!   operation, the HTTP status and a fixed error class. The API key lives in
//!   [`Secret`] (redacted `Debug`) and its header is marked sensitive.
//! - **No retries.** `book` is sent once. Any failure after sending (timeout,
//!   5xx, 408/409/425, transport error, unreadable 2xx) is
//!   [`CourierError::Timeout`]: a label may exist and the wallet may have been
//!   charged. Only a connect failure, where nothing was sent, is
//!   [`CourierError::Unavailable`].
//! - **Amounts** are naira JSON numbers that may be fractional. They are
//!   parsed from their decimal text (never through `f64` arithmetic) into
//!   kobo, exactly, with at most two decimal places and never rounded. The
//!   unit is config (`SHIPBUBBLE_AMOUNT_UNIT`).
//! - **Lenient parsing.** Ids may be strings or numbers; unknown fields are
//!   ignored; every webhook field except `event` is optional.
//! - **HTTP client.** rustls, `https_only`, no redirects, connect and total
//!   timeouts, response bodies capped at 512 KiB.

use std::{fmt, time::Duration};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use hmac::{Hmac, Mac};
use reqwest::{Method, Url, redirect::Policy};
use serde_json::{Value, json};
use sha2::Sha512;
use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    Booking, ContactAddress, CourierError, CourierEvent, CourierProvider, CourierUpdate, Parcel,
    Rate, RateQuote, ShipmentState,
};
use crate::delivery::area::AreaCouncil;
pub use crate::payments::mono::{AmountUnit, Secret};

pub const SHIPBUBBLE_NAME: &str = "shipbubble";
/// Default API base URL (`SHIPBUBBLE_BASE_URL` overrides it).
pub const DEFAULT_BASE_URL: &str = "https://api.shipbubble.com/v1/shipping";
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest response body read; anything longer is treated like a failed read.
pub const MAX_RESPONSE_BODY: usize = 512 * 1024;
/// The only message a `Rejected` carries: ours, never the provider's text.
const REJECTED_MESSAGE: &str = "courier rejected the request";
pub const SIGNATURE_HEADER: &str = "x-ship-signature";

/// Shipbubble settings, from the environment (see `config.rs`).
#[derive(Debug, Clone)]
pub struct ShipbubbleConfig {
    pub api_key: Secret,
    /// `SHIPBUBBLE_BASE_URL`, default [`DEFAULT_BASE_URL`].
    pub base_url: String,
    /// Live mode (`APP_ENV=production`; the key then starts `sb_prod`).
    pub live: bool,
    pub amount_unit: AmountUnit,
    /// `SHIPBUBBLE_CONTACT_EMAIL`: sent as `email` when an address has none.
    pub contact_email: String,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    /// **Tests only** (a local mock server speaks plain http). `config.rs`
    /// always sets `false`.
    pub insecure_http_for_tests: bool,
}

// ---------------------------------------------------------------- amounts

/// A Shipbubble amount (JSON number or numeric string, in `unit`) as kobo.
/// Parsed from the decimal text, exactly: at most two decimal places in naira
/// (a whole number in kobo), non-negative, no overflow. Never rounded.
pub fn from_sb_amount(value: &Value, unit: AmountUnit) -> Result<i64, CourierError> {
    let text = match value {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().to_string(),
        _ => return Err(CourierError::Inconclusive),
    };
    parse_decimal(&text, unit).ok_or(CourierError::Inconclusive)
}

fn parse_decimal(text: &str, unit: AmountUnit) -> Option<i64> {
    let (whole, frac) = match text.split_once('.') {
        Some((w, f)) if !f.is_empty() => (w, f),
        Some(_) => return None,
        None => (text, ""),
    };
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    let (scale, places) = match unit {
        AmountUnit::Kobo => (1, 0),
        AmountUnit::Naira => (100, 2),
    };
    // Digits beyond what the unit can hold must be zero.
    if frac.bytes().skip(places).any(|b| b != b'0') {
        return None;
    }
    let mut minor: i64 = 0;
    for i in 0..places {
        let digit = frac.as_bytes().get(i).map_or(0, |b| i64::from(b - b'0'));
        minor = minor * 10 + digit;
    }
    whole.checked_mul(scale)?.checked_add(minor)
}

/// Our kobo amount in `unit`, as the decimal string Shipbubble takes. Exact:
/// naira shows kobo as up to two decimals, kobo is a whole number. Negative is
/// refused.
pub fn to_sb_amount(kobo: i64, unit: AmountUnit) -> Result<String, CourierError> {
    if kobo < 0 {
        return Err(rejected_locally("invalid_amount"));
    }
    Ok(match unit {
        AmountUnit::Kobo => kobo.to_string(),
        AmountUnit::Naira if kobo % 100 == 0 => (kobo / 100).to_string(),
        AmountUnit::Naira => format!("{}.{:02}", kobo / 100, kobo % 100)
            .trim_end_matches('0')
            .to_string(),
    })
}

/// Grams as a kilogram string ("0.5", "2", "0.002").
fn kg_string(grams: i32) -> Result<String, CourierError> {
    if grams < 0 {
        return Err(rejected_locally("invalid_weight"));
    }
    let (kg, rem) = (grams / 1000, grams % 1000);
    Ok(if rem == 0 {
        kg.to_string()
    } else {
        format!("{kg}.{rem:03}").trim_end_matches('0').to_string()
    })
}

// ---------------------------------------------------------------- client

/// Whether a call can have taken effect at Shipbubble when it fails after sending.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Call {
    /// No side effects worth guarding (GET, address validation, rates): a
    /// failure after sending is simply "not done".
    Read,
    /// Books or cancels: a failure after sending leaves the outcome unknown.
    Write,
}

struct Answer {
    status: StatusCode,
    body: Option<Value>,
}

pub struct ShipbubbleCourier {
    cfg: ShipbubbleConfig,
    base: Url,
    client: reqwest::Client,
    key_header: HeaderValue,
}

impl fmt::Debug for ShipbubbleCourier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShipbubbleCourier")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

impl ShipbubbleCourier {
    pub fn new(cfg: ShipbubbleConfig) -> anyhow::Result<Self> {
        let base = Url::parse(cfg.base_url.trim_end_matches('/'))
            .ok()
            .filter(|u| !u.cannot_be_a_base() && u.query().is_none())
            .context("SHIPBUBBLE_BASE_URL is not a valid base URL")?;
        let mut key_header = HeaderValue::from_str(&format!("Bearer {}", cfg.api_key.expose()))
            .map_err(|_| anyhow!("SHIPBUBBLE_API_KEY contains invalid characters"))?;
        key_header.set_sensitive(true);
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(cfg.connect_timeout)
            .timeout(cfg.timeout)
            .redirect(Policy::none())
            .https_only(!cfg.insecure_http_for_tests)
            .build()
            .map_err(|e| anyhow!(e.without_url()))
            .context("build Shipbubble HTTP client")?;
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
    ) -> Result<Answer, CourierError> {
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
                provider = SHIPBUBBLE_NAME,
                op,
                cause = "body_too_large",
                "shipbubble call failed"
            );
            match call {
                Call::Write => CourierError::Timeout,
                Call::Read => CourierError::Unavailable,
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

    /// The `data` object of a 2xx answer.
    fn data<'a>(
        op: &'static str,
        call: Call,
        answer: &'a Answer,
    ) -> Result<&'a Value, CourierError> {
        if !answer.status.is_success() {
            return Err(status_error(op, call, answer));
        }
        answer
            .body
            .as_ref()
            .and_then(|b| b.get("data"))
            .filter(|d| d.is_object())
            .ok_or_else(|| unparseable(op, call))
    }
}

fn rejected_locally(code: &str) -> CourierError {
    CourierError::Rejected {
        code: code.to_string(),
        message: REJECTED_MESSAGE.into(),
    }
}

/// A 2xx body we cannot read: for a write the outcome is unknown.
fn unparseable(op: &'static str, call: Call) -> CourierError {
    tracing::warn!(
        provider = SHIPBUBBLE_NAME,
        op,
        cause = "unparseable_body",
        "shipbubble call failed"
    );
    match call {
        Call::Write => CourierError::Timeout,
        Call::Read => CourierError::Inconclusive,
    }
}

/// Maps a reqwest failure. Logs only the sanitised class, never the error
/// (it may hold the URL, which carries order ids).
fn transport_error(op: &'static str, call: Call, e: reqwest::Error) -> CourierError {
    let e = e.without_url();
    // Checked first: a connect failure means nothing was sent.
    let (cause, err) = if e.is_connect() {
        ("connect", CourierError::Unavailable)
    } else if e.is_timeout() && call == Call::Write {
        ("timeout", CourierError::Timeout)
    } else if e.is_timeout() {
        ("timeout", CourierError::Unavailable)
    } else if call == Call::Write {
        ("transport_after_send", CourierError::Timeout)
    } else {
        ("transport", CourierError::Unavailable)
    };
    tracing::warn!(
        provider = SHIPBUBBLE_NAME,
        op,
        cause,
        "shipbubble call failed"
    );
    err
}

/// Maps a non-2xx answer by HTTP status only, never by message text.
/// - 429: not processed -> Unavailable.
/// - 408/409/425, 5xx, 3xx: a write may have taken effect -> Timeout; a read
///   -> Unavailable.
/// - any other 4xx (400 included, e.g. low wallet balance or "already
///   processed") -> Rejected with code `http_<status>`.
fn status_error(op: &'static str, call: Call, answer: &Answer) -> CourierError {
    let status = answer.status.as_u16();
    tracing::warn!(
        provider = SHIPBUBBLE_NAME,
        op,
        status,
        cause = "http_status",
        "shipbubble call failed"
    );
    let unsettled = || match call {
        Call::Write => CourierError::Timeout,
        Call::Read => CourierError::Unavailable,
    };
    if answer.status == StatusCode::TOO_MANY_REQUESTS {
        CourierError::Unavailable
    } else if matches!(status, 408 | 409 | 425) {
        unsettled()
    } else if answer.status.is_client_error() {
        CourierError::Rejected {
            code: format!("http_{status}"),
            message: REJECTED_MESSAGE.into(),
        }
    } else {
        unsettled()
    }
}

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// A string-or-number field as a non-empty string (ids and phones arrive both ways).
fn text_at(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn area_label(area: AreaCouncil) -> &'static str {
    match area {
        AreaCouncil::Amac => "AMAC",
        AreaCouncil::Bwari => "Bwari",
        AreaCouncil::Gwagwalada => "Gwagwalada",
        AreaCouncil::Kuje => "Kuje",
        AreaCouncil::Kwali => "Kwali",
        AreaCouncil::Abaji => "Abaji",
    }
}

/// Whether Shipbubble's echoed `state` fields look like the FCT (log-only
/// check). **Assumed** values: `state_code` "FC" or a `state` naming the
/// Federal Capital Territory.
fn state_is_fct(data: &Value) -> bool {
    if str_at(data, "state_code").is_some_and(|c| c.trim().eq_ignore_ascii_case("FC")) {
        return true;
    }
    str_at(data, "state").is_some_and(|s| {
        let s = s.trim().to_ascii_lowercase();
        s == "fct" || s.contains("federal capital")
    })
}

/// One `couriers[]` entry; the error is a fixed cause string.
fn parse_rate(c: &Value, unit: AmountUnit) -> Result<Rate, &'static str> {
    if str_at(c, "currency").is_some_and(|cur| !cur.eq_ignore_ascii_case("NGN")) {
        return Err("non_ngn_currency");
    }
    let amount = |key: &str| {
        c.get(key)
            .ok_or("missing_amount")
            .and_then(|v| from_sb_amount(v, unit).map_err(|_| "bad_amount"))
    };
    Ok(Rate {
        courier_id: text_at(c, "courier_id").ok_or("missing_courier_id")?,
        service_code: text_at(c, "service_code").ok_or("missing_service_code")?,
        courier_name: str_at(c, "courier_name").unwrap_or_default().to_string(),
        amount_kobo: amount("rate_card_amount")?,
        charged_kobo: amount("total")?,
        delivery_eta: str_at(c, "delivery_eta").map(str::to_string),
    })
}

fn state_of(raw: &str) -> Option<ShipmentState> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "pending" => ShipmentState::Pending,
        "confirmed" => ShipmentState::Confirmed,
        "picked_up" => ShipmentState::PickedUp,
        "in_transit" => ShipmentState::InTransit,
        "completed" => ShipmentState::Completed,
        "cancelled" => ShipmentState::Cancelled,
        _ => return None,
    })
}

/// Hex decode (no dependency for it); `None` on odd length or a non-hex digit.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Logged name of a wallet event: only names we know.
fn loggable_wallet_event(event: &str) -> &'static str {
    match event {
        "wallet.threshold.alert" => "wallet.threshold.alert",
        "wallet.auto_topup.failed" => "wallet.auto_topup.failed",
        _ => "wallet.other",
    }
}

#[async_trait]
impl CourierProvider for ShipbubbleCourier {
    fn name(&self) -> &'static str {
        SHIPBUBBLE_NAME
    }

    async fn validate_address(&self, addr: &ContactAddress) -> Result<i64, CourierError> {
        const OP: &str = "validate_address";
        // Shipbubble requires an email; we do not collect buyers' emails, so the
        // platform contact address is sent when there is none. Never logged.
        let email = addr
            .email
            .as_deref()
            .unwrap_or(self.cfg.contact_email.as_str());
        let body = json!({
            "name": addr.name,
            "email": email,
            "phone": addr.phone,
            "address": format!(
                "{}, {}, Federal Capital Territory, Nigeria",
                addr.address_line,
                area_label(addr.area)
            ),
        });
        let url = self.url(&["address", "validate"]);
        let answer = self
            .call(OP, Call::Read, Method::POST, url, Some(&body))
            .await?;
        let data = Self::data(OP, Call::Read, &answer)?;
        // The FCT is gated by `AreaCouncil` before any courier call; the echoed
        // state is only logged (no address content) when it looks non-FCT.
        if !state_is_fct(data) {
            tracing::warn!(
                provider = SHIPBUBBLE_NAME,
                op = OP,
                cause = "state_mismatch",
                "shipbubble echoed a non-FCT state"
            );
        }
        text_at(data, "address_code")
            .and_then(|c| c.parse::<i64>().ok())
            .ok_or_else(|| unparseable(OP, Call::Read))
    }

    async fn quote(
        &self,
        sender_code: i64,
        receiver_code: i64,
        pickup: Date,
        parcel: &Parcel,
    ) -> Result<RateQuote, CourierError> {
        const OP: &str = "fetch_rates";
        let unit = self.cfg.amount_unit;
        let mut items = Vec::with_capacity(parcel.items.len());
        for it in &parcel.items {
            // Shipbubble takes all three numerics as strings (weight in kg).
            items.push(json!({
                "name": it.name,
                "description": it.name,
                "unit_weight": kg_string(it.unit_weight_grams)?,
                "unit_amount": to_sb_amount(it.unit_amount_kobo, unit)?,
                "quantity": it.quantity.to_string(),
            }));
        }
        let body = json!({
            "sender_address_code": sender_code,
            // Misspelled by Shipbubble.
            "reciever_address_code": receiver_code,
            "pickup_date": format!(
                "{:04}-{:02}-{:02}",
                pickup.year(),
                u8::from(pickup.month()),
                pickup.day()
            ),
            "category_id": parcel.category_id,
            "package_items": items,
            "package_dimension": {
                "length": parcel.length_cm,
                "width": parcel.width_cm,
                "height": parcel.height_cm,
            },
        });
        let url = self.url(&["fetch_rates"]);
        let answer = self
            .call(OP, Call::Read, Method::POST, url, Some(&body))
            .await?;
        let data = Self::data(OP, Call::Read, &answer)?;
        let request_token = str_at(data, "request_token")
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| unparseable(OP, Call::Read))?
            .to_string();
        let couriers = data
            .get("couriers")
            .and_then(Value::as_array)
            .ok_or_else(|| unparseable(OP, Call::Read))?;
        let mut rates = Vec::with_capacity(couriers.len());
        for c in couriers {
            // An unreadable entry is dropped (cause logged, never the body).
            match parse_rate(c, unit) {
                Ok(rate) => rates.push(rate),
                Err(cause) => tracing::warn!(
                    provider = SHIPBUBBLE_NAME,
                    op = OP,
                    cause,
                    "shipbubble rate entry dropped"
                ),
            }
        }
        if rates.is_empty() && !couriers.is_empty() {
            return Err(unparseable(OP, Call::Read));
        }
        Ok(RateQuote {
            request_token,
            rates,
        })
    }

    async fn book(
        &self,
        request_token: &str,
        courier_id: &str,
        service_code: &str,
    ) -> Result<Booking, CourierError> {
        const OP: &str = "book";
        let body = json!({
            "request_token": request_token,
            "service_code": service_code,
            "courier_id": courier_id,
        });
        let url = self.url(&["labels"]);
        let answer = self
            .call(OP, Call::Write, Method::POST, url, Some(&body))
            .await?;
        let data = Self::data(OP, Call::Write, &answer)?;
        // A label may exist: no order id in a 2xx is an unknown outcome.
        let provider_order_id =
            text_at(data, "order_id").ok_or_else(|| unparseable(OP, Call::Write))?;
        let courier = data.get("courier");
        // Only a cross-check of the quote, so an unreadable fee is None.
        let charged_kobo = data
            .get("payment")
            .and_then(|p: &Value| p.get("shipping_fee"))
            .and_then(|v| from_sb_amount(v, self.cfg.amount_unit).ok());
        if charged_kobo.is_none() {
            tracing::warn!(
                provider = SHIPBUBBLE_NAME,
                op = OP,
                cause = "bad_shipping_fee",
                "shipbubble booking fee unreadable"
            );
        }
        Ok(Booking {
            provider_order_id,
            tracking_url: text_at(data, "tracking_url"),
            courier_name: courier
                .and_then(|c| str_at(c, "name"))
                .unwrap_or_default()
                .to_string(),
            courier_phone: courier.and_then(|c| text_at(c, "phone")),
            charged_kobo,
        })
    }

    async fn cancel(&self, provider_order_id: &str) -> Result<(), CourierError> {
        const OP: &str = "cancel";
        let url = self.url(&["labels", "cancel", provider_order_id]);
        let answer = self.call(OP, Call::Write, Method::POST, url, None).await?;
        if answer.status.is_success() {
            Ok(())
        } else {
            Err(status_error(OP, Call::Write, &answer))
        }
    }

    async fn status(&self, provider_order_id: &str) -> Result<CourierUpdate, CourierError> {
        const OP: &str = "status";
        let url = self.url(&["labels", "list", provider_order_id]);
        let answer = self.call(OP, Call::Read, Method::GET, url, None).await?;
        if answer.status == StatusCode::NOT_FOUND {
            tracing::warn!(
                provider = SHIPBUBBLE_NAME,
                op = OP,
                status = 404,
                cause = "http_status",
                "shipbubble call failed"
            );
            return Err(rejected_locally("not_found"));
        }
        let data = Self::data(OP, Call::Read, &answer)?;
        let results = data
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| unparseable(OP, Call::Read))?;
        let Some(first) = results.first() else {
            return Err(rejected_locally("not_found"));
        };
        let state = str_at(first, "status")
            .and_then(state_of)
            .ok_or_else(|| unparseable(OP, Call::Read))?;
        let courier = first.get("courier");
        Ok(CourierUpdate {
            provider_order_id: text_at(first, "order_id")
                .unwrap_or_else(|| provider_order_id.to_string()),
            state,
            occurred_at: None,
            courier_phone: courier.and_then(|c| text_at(c, "phone")),
            tracking_url: text_at(first, "tracking_url"),
        })
    }

    fn parse_webhook(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<CourierEvent, CourierError> {
        // Signature first, over the raw bytes, constant time (`verify_slice`).
        let given = headers
            .get(SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(hex_decode)
            .ok_or(CourierError::InvalidSignature)?;
        let mut mac = Hmac::<Sha512>::new_from_slice(self.cfg.api_key.expose().as_bytes())
            .map_err(|_| CourierError::InvalidSignature)?;
        mac.update(body);
        mac.verify_slice(&given)
            .map_err(|_| CourierError::InvalidSignature)?;

        let payload: Value = serde_json::from_slice(body).map_err(|_| {
            tracing::warn!(
                provider = SHIPBUBBLE_NAME,
                cause = "unparseable_webhook",
                "shipbubble webhook ignored"
            );
            CourierError::Inconclusive
        })?;
        let event = str_at(&payload, "event").ok_or(CourierError::Inconclusive)?;
        match event {
            "shipment.status.changed" | "shipment.cancelled" => {}
            // A successful top-up is good news, not a warning.
            "wallet.auto_topup.succeeded" => return Ok(CourierEvent::Ignored),
            e if e.starts_with("wallet.") => {
                let event = loggable_wallet_event(e);
                tracing::warn!(provider = SHIPBUBBLE_NAME, event, "wallet_alert");
                return Ok(CourierEvent::WalletAlert {
                    event: event.to_string(),
                });
            }
            // label.created, sla.updated, cod/waybill events and anything new.
            _ => return Ok(CourierEvent::Ignored),
        }
        let Some(provider_order_id) = text_at(&payload, "order_id") else {
            tracing::warn!(
                provider = SHIPBUBBLE_NAME,
                cause = "missing_order_id",
                "shipbubble webhook ignored"
            );
            return Ok(CourierEvent::Ignored);
        };
        let state = match str_at(&payload, "status") {
            Some(raw) => match state_of(raw) {
                Some(s) => s,
                None => {
                    tracing::warn!(
                        provider = SHIPBUBBLE_NAME,
                        cause = "unknown_status",
                        "shipbubble webhook ignored"
                    );
                    return Ok(CourierEvent::Ignored);
                }
            },
            None if event == "shipment.cancelled" => ShipmentState::Cancelled,
            None => {
                tracing::warn!(
                    provider = SHIPBUBBLE_NAME,
                    cause = "missing_status",
                    "shipbubble webhook ignored"
                );
                return Ok(CourierEvent::Ignored);
            }
        };
        Ok(CourierEvent::Update(CourierUpdate {
            provider_order_id,
            state,
            occurred_at: str_at(&payload, "date")
                .and_then(|d| OffsetDateTime::parse(d.trim(), &Rfc3339).ok()),
            courier_phone: payload.get("courier").and_then(|c| text_at(c, "phone")),
            tracking_url: text_at(&payload, "tracking_url"),
        }))
    }
}
