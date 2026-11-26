//! The partner-services seam (part 9): one safe, switchable, paid-for way to call a service
//! built by the owner's other company. No capability is built here; the partner's contract
//! does not exist yet.
//!
//! Rules this module keeps:
//! - **The marketplace never depends on the partner.** With no partner configured, the
//!   switch off, the budget spent, the breaker open or the partner down, every call answers
//!   a [`PartnerError`] the caller handles, and nothing else in the app changes.
//! - **Data minimisation.** Only the fields in [`client::ALLOWED_OUTBOUND_FIELDS`] may be
//!   sent: opaque ids, barcodes, prices in integer kobo and timestamps. A payload with any
//!   other field is refused before a network call.
//! - **Secrets** live in [`Secret`] (redacted `Debug`) and are never logged, put in an
//!   error or an audit row. Errors carry a fixed code, never the partner's text.
//! - **We are the paying customer.** Every call is metered per feature and day, and a
//!   monthly budget stops calls when it is spent.

pub mod breaker;
pub mod client;
pub mod config;
pub mod fake;
pub mod gateway;
pub mod jobs;
pub mod routes;
pub mod settings;
pub mod usage;
pub mod webhook;

pub use config::PartnerConfig;
pub use gateway::PartnerGateway;

/// Why a call did not produce an answer. No partner text, URL or key is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartnerError {
    /// No partner is configured.
    NotConfigured,
    /// The Owner has the partner switched off.
    Disabled,
    /// This month's call budget is spent.
    BudgetSpent,
    /// The breaker is open after repeated failures.
    CircuitOpen,
    /// The payload held a field that may not be sent.
    Blocked,
    /// The partner did not answer in time or could not be reached.
    Unavailable,
    /// The partner answered with an error.
    Rejected,
    /// The answer was larger than the cap or not valid JSON.
    BadResponse,
}

impl PartnerError {
    /// A fixed code for logs, the dashboard and the audit trail.
    pub fn code(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::Disabled => "disabled",
            Self::BudgetSpent => "budget_spent",
            Self::CircuitOpen => "circuit_open",
            Self::Blocked => "blocked",
            Self::Unavailable => "unavailable",
            Self::Rejected => "rejected",
            Self::BadResponse => "bad_response",
        }
    }
}

/// A feature name: 1 to 40 characters of lowercase letters, digits and `_`.
pub fn valid_feature(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
