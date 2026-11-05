//! Test double for [`ToneChecker`]. Answers clean unless told otherwise.

use std::{collections::VecDeque, sync::Mutex};

use async_trait::async_trait;

use super::tone::{ToneChecker, ToneError, ToneVerdict};

type Answer = Result<ToneVerdict, ToneError>;

#[derive(Default)]
pub struct FakeTone {
    next: Mutex<VecDeque<Answer>>,
    all: Mutex<Option<Answer>>,
    seen: Mutex<Vec<String>>,
}

impl FakeTone {
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

    /// Back to answering clean.
    pub fn reset(&self) {
        self.next.lock().unwrap().clear();
        *self.all.lock().unwrap() = None;
    }

    pub fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToneChecker for FakeTone {
    async fn check(&self, text: &str) -> Answer {
        self.seen.lock().unwrap().push(text.to_string());
        if let Some(answer) = self.next.lock().unwrap().pop_front() {
            return answer;
        }
        self.all
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(ToneVerdict::clean()))
    }
}
