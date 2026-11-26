//! The one door every partner call goes through: configured, switched on, within budget,
//! breaker closed, payload allowed, then the call, then metering. Nothing else in the app
//! reaches the partner, and nothing in the app waits on it beyond the client's own timeout.

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use serde_json::Value;
use sqlx::PgPool;

use super::{
    PartnerError,
    breaker::{Breaker, State},
    client::{PartnerApi, check_outbound},
    settings, usage,
};
use crate::payments::mono::Secret;

pub struct PartnerGateway {
    api: Arc<dyn PartnerApi>,
    db: PgPool,
    breaker: Breaker,
    webhook_secret: Option<Secret>,
    last_error: Mutex<Option<&'static str>>,
}

impl PartnerGateway {
    pub fn new(api: Arc<dyn PartnerApi>, db: PgPool, webhook_secret: Option<Secret>) -> Self {
        Self {
            api,
            db,
            breaker: Breaker::new(),
            webhook_secret,
            last_error: Mutex::new(None),
        }
    }

    pub fn configured(&self) -> bool {
        self.api.configured()
    }

    pub fn webhook_secret(&self) -> Option<&Secret> {
        self.webhook_secret.as_ref()
    }

    /// The fixed code of the last failure, for the dashboard; `None` after a success.
    pub fn last_error(&self) -> Option<&'static str> {
        *self.last_error.lock().unwrap()
    }

    pub fn breaker_state(&self) -> State {
        self.breaker.state(Instant::now())
    }

    fn note(&self, e: Option<&'static str>) {
        *self.last_error.lock().unwrap() = e;
    }

    /// Whether partner-powered features may show right now: configured, switched on, the
    /// budget not spent and the breaker closed. A feature is also off in its own flag.
    pub async fn available(&self) -> Result<bool, crate::error::AppError> {
        if !self.configured() {
            return Ok(false);
        }
        let s = settings::load(&self.db).await?;
        Ok(s.enabled
            && self.breaker_state() == State::Closed
            && !usage::spent(&self.db, s.monthly_call_budget).await?)
    }

    async fn gate(&self, feature: &str) -> Result<(), PartnerError> {
        debug_assert!(super::valid_feature(feature));
        if !self.configured() {
            return Err(PartnerError::NotConfigured);
        }
        let s = settings::load(&self.db)
            .await
            .map_err(|_| PartnerError::Unavailable)?;
        if !s.enabled {
            return Err(PartnerError::Disabled);
        }
        if usage::spent(&self.db, s.monthly_call_budget)
            .await
            .map_err(|_| PartnerError::Unavailable)?
        {
            // One alert a month, however many calls are refused.
            let _ = usage::raise_budget_alert(&self.db).await;
            return Err(PartnerError::BudgetSpent);
        }
        if !self.breaker.allow(Instant::now()) {
            return Err(PartnerError::CircuitOpen);
        }
        Ok(())
    }

    async fn finish(
        &self,
        feature: &str,
        result: Result<Value, PartnerError>,
    ) -> Result<Value, PartnerError> {
        match &result {
            Ok(_) => {
                self.breaker.success();
                self.note(None);
            }
            // The partner said no to this request: it is up, so the breaker stays calm.
            Err(PartnerError::Rejected) => {
                self.breaker.success();
                self.note(Some(PartnerError::Rejected.code()));
            }
            Err(e) => {
                self.breaker.failure(Instant::now());
                self.note(Some(e.code()));
            }
        }
        // Paid for, whatever the answer; never inside a transaction.
        let _ = usage::record(&self.db, feature, result.is_err()).await;
        result
    }

    pub async fn get(&self, feature: &str, path: &str) -> Result<Value, PartnerError> {
        self.gate(feature).await?;
        let r = self.api.get(path).await;
        self.finish(feature, r).await
    }

    pub async fn post(
        &self,
        feature: &str,
        path: &str,
        idempotency_key: &str,
        body: &Value,
    ) -> Result<Value, PartnerError> {
        // Before any gate, so a bad payload costs nothing and trips nothing.
        check_outbound(body)?;
        self.gate(feature).await?;
        let r = self.api.post(path, idempotency_key, body).await;
        self.finish(feature, r).await
    }

    /// The connection check for the dashboard. It is not metered and ignores the switch and
    /// the budget, but it respects the breaker.
    pub async fn health(&self) -> Result<Value, PartnerError> {
        if !self.configured() {
            return Err(PartnerError::NotConfigured);
        }
        if !self.breaker.allow(Instant::now()) {
            return Err(PartnerError::CircuitOpen);
        }
        let r = self.api.get("v1/health").await;
        match &r {
            Ok(_) | Err(PartnerError::Rejected) => {
                self.breaker.success();
                self.note(None);
            }
            Err(e) => {
                self.breaker.failure(Instant::now());
                self.note(Some(e.code()));
            }
        }
        r
    }
}
