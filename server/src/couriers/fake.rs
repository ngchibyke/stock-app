//! A programmable [`CourierProvider`] test double. **Test and development
//! only:** production never registers it. It is compiled into the library (not
//! `cfg(test)`) because the integration tests in `tests/` use it.
//!
//! Defaults: every call succeeds. `validate_address` returns an incrementing
//! code, `quote` one default rate (unless [`FakeCourier::set_rates`]), `book`
//! returns `fake-<n>`, `status` returns the next pushed update (or `Pending`).

use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use axum::http::HeaderMap;
use serde::Deserialize;
use time::{Date, OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    Booking, ContactAddress, CourierError, CourierEvent, CourierProvider, CourierUpdate, Parcel,
    Rate, RateQuote, ShipmentState,
};

pub const FAKE_COURIER_NAME: &str = "fake";
pub const FAKE_SIGNATURE_HEADER: &str = "x-fake-signature";

/// `CourierError` is not `Clone`; a stored outcome is re-created on every call.
fn copy_err(e: &CourierError) -> CourierError {
    match e {
        CourierError::Rejected { code, message } => CourierError::Rejected {
            code: code.clone(),
            message: message.clone(),
        },
        CourierError::Timeout => CourierError::Timeout,
        CourierError::Unavailable => CourierError::Unavailable,
        CourierError::Inconclusive => CourierError::Inconclusive,
        CourierError::InvalidSignature => CourierError::InvalidSignature,
        CourierError::NotSupported => CourierError::NotSupported,
    }
}

pub fn default_rate() -> Rate {
    Rate {
        courier_id: "fake-courier".into(),
        service_code: "fake-standard".into(),
        courier_name: "Fake Courier".into(),
        amount_kobo: 150_000,
        charged_kobo: 150_000,
        delivery_eta: Some("Within 24 hours".into()),
    }
}

pub struct FakeCourier {
    next_address_code: AtomicI64,
    next_order: AtomicUsize,
    rates: Mutex<Vec<Rate>>,
    quote_failure: Mutex<Option<CourierError>>,
    book_failure: Mutex<Option<CourierError>>,
    book_delay: Mutex<Option<Duration>>,
    booking_phone: Mutex<Option<String>>,
    booking_url: Mutex<Option<String>>,
    cancel_failure: Mutex<Option<CourierError>>,
    cancel_delay: Mutex<Option<Duration>>,
    updates: Mutex<VecDeque<CourierUpdate>>,
    book_calls: AtomicUsize,
    quote_calls: AtomicUsize,
    cancel_calls: AtomicUsize,
    status_calls: AtomicUsize,
}

impl Default for FakeCourier {
    fn default() -> Self {
        Self {
            next_address_code: AtomicI64::new(1000),
            next_order: AtomicUsize::new(1),
            rates: Mutex::new(vec![default_rate()]),
            quote_failure: Mutex::default(),
            book_failure: Mutex::default(),
            book_delay: Mutex::default(),
            booking_phone: Mutex::default(),
            booking_url: Mutex::default(),
            cancel_failure: Mutex::default(),
            cancel_delay: Mutex::default(),
            updates: Mutex::default(),
            book_calls: AtomicUsize::new(0),
            quote_calls: AtomicUsize::new(0),
            cancel_calls: AtomicUsize::new(0),
            status_calls: AtomicUsize::new(0),
        }
    }
}

impl FakeCourier {
    pub fn new() -> Self {
        Self::default()
    }

    /// The rates every later `quote` returns.
    pub fn set_rates(&self, rates: Vec<Rate>) {
        *self.rates.lock().unwrap() = rates;
    }

    /// Every later `quote` fails with this error.
    pub fn fail_quote_with(&self, err: CourierError) {
        *self.quote_failure.lock().unwrap() = Some(err);
    }

    /// Every later `book` fails with this error, until
    /// [`clear_book_failure`](Self::clear_book_failure).
    pub fn fail_book_with(&self, err: CourierError) {
        *self.book_failure.lock().unwrap() = Some(err);
    }

    pub fn clear_book_failure(&self) {
        *self.book_failure.lock().unwrap() = None;
    }

    /// The next `book` call sleeps this long before answering (once).
    pub fn delay_next_book(&self, d: Duration) {
        *self.book_delay.lock().unwrap() = Some(d);
    }

    /// The courier phone every later `book` reports.
    /// The tracking URL bookings answer with, in place of the default.
    pub fn set_booking_url(&self, url: &str) {
        *self.booking_url.lock().unwrap() = Some(url.to_string());
    }

    pub fn set_booking_phone(&self, phone: Option<String>) {
        *self.booking_phone.lock().unwrap() = phone;
    }

    /// Every later `cancel` fails with this error.
    pub fn fail_cancel_with(&self, err: CourierError) {
        *self.cancel_failure.lock().unwrap() = Some(err);
    }

    pub fn clear_cancel_failure(&self) {
        *self.cancel_failure.lock().unwrap() = None;
    }

    /// The next `cancel` call sleeps this long before answering (once).
    pub fn delay_next_cancel(&self, d: Duration) {
        *self.cancel_delay.lock().unwrap() = Some(d);
    }

    /// Queues what `status()` returns next (FIFO; the last queued update keeps
    /// repeating once the queue is down to one).
    pub fn push_update(&self, update: CourierUpdate) {
        self.updates.lock().unwrap().push_back(update);
    }

    pub fn book_calls(&self) -> usize {
        self.book_calls.load(Ordering::SeqCst)
    }
    pub fn quote_calls(&self) -> usize {
        self.quote_calls.load(Ordering::SeqCst)
    }
    pub fn cancel_calls(&self) -> usize {
        self.cancel_calls.load(Ordering::SeqCst)
    }
    pub fn status_calls(&self) -> usize {
        self.status_calls.load(Ordering::SeqCst)
    }
}

#[derive(Deserialize)]
struct FakeWebhook {
    /// `wallet.*` makes it a wallet alert; the other fields are then unused.
    event: Option<String>,
    provider_order_id: Option<String>,
    state: Option<String>,
    occurred_at: Option<String>,
    courier_phone: Option<String>,
    tracking_url: Option<String>,
}

fn parse_state(s: &str) -> Option<ShipmentState> {
    Some(match s {
        "pending" => ShipmentState::Pending,
        "confirmed" => ShipmentState::Confirmed,
        "picked_up" => ShipmentState::PickedUp,
        "in_transit" => ShipmentState::InTransit,
        "completed" => ShipmentState::Completed,
        "cancelled" => ShipmentState::Cancelled,
        _ => return None,
    })
}

#[async_trait]
impl CourierProvider for FakeCourier {
    fn name(&self) -> &'static str {
        FAKE_COURIER_NAME
    }

    async fn validate_address(&self, _addr: &ContactAddress) -> Result<i64, CourierError> {
        Ok(self.next_address_code.fetch_add(1, Ordering::SeqCst))
    }

    async fn quote(
        &self,
        _sender_code: i64,
        _receiver_code: i64,
        _pickup: Date,
        _parcel: &Parcel,
    ) -> Result<RateQuote, CourierError> {
        self.quote_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(e) = &*self.quote_failure.lock().unwrap() {
            return Err(copy_err(e));
        }
        Ok(RateQuote {
            request_token: "fake-token".into(),
            rates: self.rates.lock().unwrap().clone(),
        })
    }

    async fn book(
        &self,
        _request_token: &str,
        _courier_id: &str,
        _service_code: &str,
    ) -> Result<Booking, CourierError> {
        self.book_calls.fetch_add(1, Ordering::SeqCst);
        let delay = self.book_delay.lock().unwrap().take();
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
        if let Some(e) = &*self.book_failure.lock().unwrap() {
            return Err(copy_err(e));
        }
        let n = self.next_order.fetch_add(1, Ordering::SeqCst);
        let rate = self.rates.lock().unwrap().first().cloned();
        Ok(Booking {
            provider_order_id: format!("fake-{n}"),
            tracking_url: Some(
                self.booking_url
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| format!("https://track.fake.test/fake-{n}")),
            ),
            courier_name: rate
                .as_ref()
                .map_or_else(|| "Fake Courier".into(), |r| r.courier_name.clone()),
            courier_phone: self.booking_phone.lock().unwrap().clone(),
            charged_kobo: rate.map(|r| r.charged_kobo),
        })
    }

    async fn cancel(&self, _provider_order_id: &str) -> Result<(), CourierError> {
        self.cancel_calls.fetch_add(1, Ordering::SeqCst);
        let delay = self.cancel_delay.lock().unwrap().take();
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
        match &*self.cancel_failure.lock().unwrap() {
            Some(e) => Err(copy_err(e)),
            None => Ok(()),
        }
    }

    async fn status(&self, provider_order_id: &str) -> Result<CourierUpdate, CourierError> {
        self.status_calls.fetch_add(1, Ordering::SeqCst);
        let mut q = self.updates.lock().unwrap();
        let update = if q.len() > 1 {
            q.pop_front()
        } else {
            q.front().cloned()
        };
        Ok(update.unwrap_or_else(|| CourierUpdate {
            provider_order_id: provider_order_id.to_string(),
            state: ShipmentState::Pending,
            occurred_at: None,
            courier_phone: None,
            tracking_url: None,
        }))
    }

    fn parse_webhook(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<CourierEvent, CourierError> {
        let ok = headers
            .get(FAKE_SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == "ok");
        if !ok {
            return Err(CourierError::InvalidSignature);
        }
        let w: FakeWebhook =
            serde_json::from_slice(body).map_err(|_| CourierError::Inconclusive)?;
        if let Some(event) = w.event.filter(|e| e.starts_with("wallet.")) {
            return Ok(CourierEvent::WalletAlert { event });
        }
        let (Some(provider_order_id), Some(state)) = (w.provider_order_id, w.state) else {
            return Err(CourierError::Inconclusive);
        };
        let Some(state) = parse_state(&state) else {
            return Ok(CourierEvent::Ignored);
        };
        let occurred_at = w
            .occurred_at
            .and_then(|s| OffsetDateTime::parse(&s, &Rfc3339).ok());
        Ok(CourierEvent::Update(CourierUpdate {
            provider_order_id,
            state,
            occurred_at,
            courier_phone: w.courier_phone,
            tracking_url: w.tracking_url,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::couriers::parcel_for;
    use crate::delivery::area::AreaCouncil;

    fn addr() -> ContactAddress {
        ContactAddress {
            name: "A".into(),
            email: None,
            phone: "0".into(),
            address_line: "x".into(),
            area: AreaCouncil::Amac,
        }
    }

    #[tokio::test]
    async fn fake_records_calls() {
        let f = FakeCourier::new();
        let a = f.validate_address(&addr()).await.unwrap();
        let b = f.validate_address(&addr()).await.unwrap();
        assert_eq!(b, a + 1);

        let today = Date::from_calendar_date(2026, time::Month::November, 3).unwrap();
        let q = f.quote(a, b, today, &parcel_for(&[], 1)).await.unwrap();
        assert_eq!(q.rates.len(), 1);
        assert_eq!(f.quote_calls(), 1);

        let r = &q.rates[0];
        let b1 = f
            .book(&q.request_token, &r.courier_id, &r.service_code)
            .await
            .unwrap();
        let b2 = f
            .book(&q.request_token, &r.courier_id, &r.service_code)
            .await
            .unwrap();
        assert_eq!(b1.provider_order_id, "fake-1");
        assert_eq!(b2.provider_order_id, "fake-2");
        assert_eq!(f.book_calls(), 2);

        f.fail_book_with(CourierError::Timeout);
        assert!(matches!(
            f.book("t", "c", "s").await,
            Err(CourierError::Timeout)
        ));
        assert_eq!(f.book_calls(), 3);

        f.cancel("fake-1").await.unwrap();
        assert_eq!(f.cancel_calls(), 1);

        f.push_update(CourierUpdate {
            provider_order_id: "fake-1".into(),
            state: ShipmentState::PickedUp,
            occurred_at: None,
            courier_phone: None,
            tracking_url: None,
        });
        assert_eq!(
            f.status("fake-1").await.unwrap().state,
            ShipmentState::PickedUp
        );
        assert_eq!(f.status_calls(), 1);
    }

    #[test]
    fn fake_webhook_needs_signature() {
        let f = FakeCourier::new();
        let body = br#"{"provider_order_id":"fake-1","state":"completed"}"#;
        assert!(matches!(
            f.parse_webhook(&HeaderMap::new(), body),
            Err(CourierError::InvalidSignature)
        ));
        let mut h = HeaderMap::new();
        h.insert(FAKE_SIGNATURE_HEADER, "ok".parse().unwrap());
        let CourierEvent::Update(u) = f.parse_webhook(&h, body).unwrap() else {
            panic!("an update");
        };
        assert_eq!(u.state, ShipmentState::Completed);
        let unknown = br#"{"provider_order_id":"fake-1","state":"returned"}"#;
        assert_eq!(f.parse_webhook(&h, unknown).unwrap(), CourierEvent::Ignored);
        assert_eq!(
            f.parse_webhook(&h, br#"{"event":"wallet.threshold.alert"}"#)
                .unwrap(),
            CourierEvent::WalletAlert {
                event: "wallet.threshold.alert".into()
            }
        );
    }
}
