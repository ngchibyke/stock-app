//! Test double for [`PartnerApi`]. Answers `{}` unless told otherwise, records every call
//! (path and, for a `POST`, the body and the idempotency key), and can be made slow.

use std::{collections::VecDeque, sync::Mutex, time::Duration};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{PartnerError, client::PartnerApi, client::check_outbound};

type Answer = Result<Value, PartnerError>;
/// path, body (a POST's), idempotency key (a POST's).
type Call = (String, Option<Value>, Option<String>);

#[derive(Default)]
pub struct FakePartner {
    next: Mutex<VecDeque<Answer>>,
    all: Mutex<Option<Answer>>,
    delay: Mutex<Option<Duration>>,
    calls: Mutex<Vec<Call>>,
}

impl FakePartner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues one answer; queued answers go first.
    pub fn set_next(&self, answer: Answer) {
        self.next.lock().unwrap().push_back(answer);
    }

    /// The answer for every call with nothing queued.
    pub fn set_all(&self, answer: Answer) {
        *self.all.lock().unwrap() = Some(answer);
    }

    pub fn set_delay(&self, delay: Option<Duration>) {
        *self.delay.lock().unwrap() = delay;
    }

    pub fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    pub fn paths(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.0.clone())
            .collect()
    }

    pub fn last_body(&self) -> Option<Value> {
        self.calls.lock().unwrap().last().and_then(|c| c.1.clone())
    }

    pub fn last_idempotency_key(&self) -> Option<String> {
        self.calls.lock().unwrap().last().and_then(|c| c.2.clone())
    }

    async fn answer(&self) -> Answer {
        let delay = *self.delay.lock().unwrap();
        if let Some(d) = delay {
            tokio::time::sleep(d).await;
        }
        if let Some(a) = self.next.lock().unwrap().pop_front() {
            return a;
        }
        self.all
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(json!({})))
    }
}

#[async_trait]
impl PartnerApi for FakePartner {
    fn configured(&self) -> bool {
        true
    }

    async fn get(&self, path: &str) -> Answer {
        self.calls
            .lock()
            .unwrap()
            .push((path.to_string(), None, None));
        self.answer().await
    }

    async fn post(&self, path: &str, idempotency_key: &str, body: &Value) -> Answer {
        // The same rule as the real client: nothing outside the allow-list leaves.
        check_outbound(body)?;
        self.calls.lock().unwrap().push((
            path.to_string(),
            Some(body.clone()),
            Some(idempotency_key.to_string()),
        ));
        self.answer().await
    }
}
