//! Test double for [`PushSender`]. Answers Ok unless told otherwise.

use std::{
    collections::{HashSet, VecDeque},
    sync::Mutex,
    time::Duration,
};

use async_trait::async_trait;
use uuid::Uuid;

use super::push::{PushError, PushMessage, PushSender};

type Answer = Result<(), PushError>;

/// What reached the fake: every field of the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub token: String,
    pub title: &'static str,
    pub body: &'static str,
    pub kind: &'static str,
    pub notification_id: Uuid,
}

#[derive(Default)]
pub struct FakePush {
    next: Mutex<VecDeque<Answer>>,
    all: Mutex<Option<Answer>>,
    dead: Mutex<HashSet<String>>,
    failing: Mutex<HashSet<String>>,
    delay: Mutex<Option<Duration>>,
    seen: Mutex<Vec<Seen>>,
}

impl FakePush {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues one answer. Queued answers go first, then [`set_all`](Self::set_all).
    pub fn set_next(&self, answer: Answer) {
        self.next.lock().unwrap().push_back(answer);
    }

    /// The answer for every call with nothing queued.
    pub fn set_all(&self, answer: Answer) {
        *self.all.lock().unwrap() = Some(answer);
    }

    /// From now on this token answers [`PushError::Unregistered`].
    pub fn unregister(&self, token: &str) {
        self.dead.lock().unwrap().insert(token.to_string());
    }

    /// This token answers [`PushError::Unavailable`] while `down`.
    pub fn set_unavailable(&self, token: &str, down: bool) {
        let mut failing = self.failing.lock().unwrap();
        if down {
            failing.insert(token.to_string());
        } else {
            failing.remove(token);
        }
    }

    /// Every call waits this long before answering.
    pub fn set_delay(&self, delay: Duration) {
        *self.delay.lock().unwrap() = Some(delay);
    }

    /// Every call made, answered Ok or not: `(token, kind, notification id)`.
    pub fn sent(&self) -> Vec<(String, &'static str, Uuid)> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|s| (s.token.clone(), s.kind, s.notification_id))
            .collect()
    }

    /// Every call made, with the whole message.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl PushSender for FakePush {
    async fn send(&self, m: &PushMessage<'_>) -> Answer {
        self.seen.lock().unwrap().push(Seen {
            token: m.token.to_string(),
            title: m.title,
            body: m.body,
            kind: m.kind,
            notification_id: m.notification_id,
        });
        let delay = *self.delay.lock().unwrap();
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        if self.dead.lock().unwrap().contains(m.token) {
            return Err(PushError::Unregistered);
        }
        if self.failing.lock().unwrap().contains(m.token) {
            return Err(PushError::Unavailable);
        }
        if let Some(answer) = self.next.lock().unwrap().pop_front() {
            return answer;
        }
        self.all.lock().unwrap().unwrap_or(Ok(()))
    }
}
