//! A circuit breaker. It opens after [`OPEN_AFTER`] failures in a row, lets one probe
//! through after [`HALF_OPEN_AFTER`], and closes again on the first success. Times are
//! passed in, so tests need no sleeping.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

pub const OPEN_AFTER: u32 = 5;
pub const HALF_OPEN_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Default)]
struct Inner {
    failures: u32,
    opened_at: Option<Instant>,
    probing: bool,
}

#[derive(Default)]
pub struct Breaker {
    inner: Mutex<Inner>,
}

impl Breaker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self, now: Instant) -> State {
        let i = self.inner.lock().unwrap();
        match i.opened_at {
            None => State::Closed,
            Some(at) if now.duration_since(at) >= HALF_OPEN_AFTER => State::HalfOpen,
            Some(_) => State::Open,
        }
    }

    /// Whether a call may go out now. While half-open, exactly one probe is let through.
    pub fn allow(&self, now: Instant) -> bool {
        let mut i = self.inner.lock().unwrap();
        match i.opened_at {
            None => true,
            Some(at) if now.duration_since(at) >= HALF_OPEN_AFTER && !i.probing => {
                i.probing = true;
                true
            }
            Some(_) => false,
        }
    }

    pub fn success(&self) {
        *self.inner.lock().unwrap() = Inner::default();
    }

    pub fn failure(&self, now: Instant) {
        let mut i = self.inner.lock().unwrap();
        i.failures += 1;
        if i.opened_at.is_some() {
            // A failed probe: open again, from now.
            i.opened_at = Some(now);
            i.probing = false;
        } else if i.failures >= OPEN_AFTER {
            i.opened_at = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_opens_after_five_failures_and_recovers_through_one_probe() {
        let b = Breaker::new();
        let t0 = Instant::now();
        for _ in 0..4 {
            b.failure(t0);
            assert_eq!(b.state(t0), State::Closed);
            assert!(b.allow(t0));
        }
        b.failure(t0);
        assert_eq!(b.state(t0), State::Open);
        assert!(!b.allow(t0 + Duration::from_secs(29)));
        let later = t0 + Duration::from_secs(31);
        assert_eq!(b.state(later), State::HalfOpen);
        // One probe, then no more until it answers.
        assert!(b.allow(later));
        assert!(!b.allow(later));
        // The probe fails: open again for another 30 seconds from now.
        b.failure(later);
        assert_eq!(b.state(later), State::Open);
        assert!(!b.allow(later + Duration::from_secs(29)));
        // The next probe succeeds: closed, and the count starts again.
        let again = later + Duration::from_secs(31);
        assert!(b.allow(again));
        b.success();
        assert_eq!(b.state(again), State::Closed);
        for _ in 0..4 {
            b.failure(again);
        }
        assert_eq!(
            b.state(again),
            State::Closed,
            "the old failures do not count"
        );
    }

    #[test]
    fn one_success_clears_the_run_of_failures() {
        let b = Breaker::new();
        let t = Instant::now();
        for _ in 0..4 {
            b.failure(t);
        }
        b.success();
        b.failure(t);
        assert_eq!(b.state(t), State::Closed);
    }
}
