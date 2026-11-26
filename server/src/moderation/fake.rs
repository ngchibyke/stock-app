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

/// A tone check for development and the app's wire check, chosen with
/// `TONE_PROVIDER=fake`. It judges by marker, not by meaning: `[[mild]]` is a rude
/// message with a suggestion, `[[serious]]` an abusive one, `[[down]]` an outage and
/// anything else is clean. `main` uses it only when no `ANTHROPIC_API_KEY` is set, and
/// production refuses to start without one, so it cannot run there.
pub struct ScriptedTone;

pub const SCRIPTED_SUGGESTION: &str = "Could we say that more kindly?";

#[async_trait]
impl ToneChecker for ScriptedTone {
    async fn check(&self, text: &str) -> Answer {
        use super::filter::Category;
        if text.contains("[[down]]") {
            Err(ToneError::Unavailable)
        } else if text.contains("[[serious]]") {
            Ok(ToneVerdict::serious(Category::Abuse))
        } else if text.contains("[[mild]]") {
            Ok(ToneVerdict::mild(SCRIPTED_SUGGESTION))
        } else {
            Ok(ToneVerdict::clean())
        }
    }
}

#[cfg(test)]
mod scripted_tests {
    use super::*;

    #[tokio::test]
    async fn markers_choose_the_verdict() {
        let t = ScriptedTone;
        assert_eq!(t.check("hello").await.unwrap(), ToneVerdict::clean());
        assert_eq!(
            t.check("[[mild]] you").await.unwrap(),
            ToneVerdict::mild(SCRIPTED_SUGGESTION)
        );
        assert!(t.check("x [[serious]]").await.unwrap().severity.is_some());
        assert_eq!(t.check("[[down]]").await, Err(ToneError::Unavailable));
    }
}
