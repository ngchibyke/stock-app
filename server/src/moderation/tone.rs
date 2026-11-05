//! The tone check. Chat and reviews talk only to [`ToneChecker`]; Claude Haiku
//! is the production implementation and `fake::FakeTone` the test double.
//!
//! Only the text being judged crosses this trait: no names, ids or history.

use std::fmt;

use async_trait::async_trait;

use super::filter::{Category, Severity};

/// What the tone check said. `severity: None` means clean; `category: None` is
/// the API's `none`. The suggestion is message text, so `Debug` hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct ToneVerdict {
    pub severity: Option<Severity>,
    pub category: Option<Category>,
    pub suggestion: Option<String>,
}

impl ToneVerdict {
    pub fn clean() -> Self {
        Self {
            severity: None,
            category: None,
            suggestion: None,
        }
    }

    pub fn mild(suggestion: &str) -> Self {
        Self {
            severity: Some(Severity::Mild),
            category: Some(Category::Rude),
            suggestion: Some(suggestion.to_string()),
        }
    }

    pub fn serious(category: Category) -> Self {
        Self {
            severity: Some(Severity::Serious),
            category: Some(category),
            suggestion: None,
        }
    }
}

impl fmt::Debug for ToneVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToneVerdict")
            .field("severity", &self.severity)
            .field("category", &self.category)
            .finish_non_exhaustive()
    }
}

/// Why no verdict came back. Carries no detail, so no provider text or message
/// text can reach a log through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneError {
    /// Worth trying again: down, slow, rate-limited or an unreadable answer.
    Unavailable,
    /// The model declined this text. Trying the same text again will not help.
    Refused,
}

#[async_trait]
pub trait ToneChecker: Send + Sync {
    async fn check(&self, text: &str) -> Result<ToneVerdict, ToneError>;
}

/// Everything passes. Only for non-production with no key: production refuses
/// to start without one.
pub struct NoTone;

#[async_trait]
impl ToneChecker for NoTone {
    async fn check(&self, _text: &str) -> Result<ToneVerdict, ToneError> {
        Ok(ToneVerdict::clean())
    }
}
