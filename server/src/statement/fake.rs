//! A programmable [`StatementSource`] test double. **Test and development
//! only:** production never installs it. Compiled into the library because the
//! integration tests use it. A day with no totals set answers "unavailable".

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use time::Date;

use super::{StatementSource, Totals};
use crate::payments::ProviderError;

#[derive(Default)]
pub struct FakeStatement {
    days: Mutex<HashMap<Date, Totals>>,
    calls: AtomicUsize,
}

impl FakeStatement {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_day(&self, day: Date, totals: Totals) {
        self.days.lock().unwrap().insert(day, totals);
    }

    pub fn clear_day(&self, day: Date) {
        self.days.lock().unwrap().remove(&day);
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl StatementSource for FakeStatement {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn day_totals(&self, day: Date) -> Result<Totals, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.days
            .lock()
            .unwrap()
            .get(&day)
            .copied()
            .ok_or_else(|| ProviderError::Unavailable(anyhow::anyhow!("no statement for the day")))
    }
}
