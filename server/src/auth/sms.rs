//! Sending sign-up and password-reset codes by SMS.
//!
//! [`SmsOtpSender`] implements [`OtpSender`] over a small [`SmsProvider`] trait, so choosing
//! a provider is one adapter. Rules it keeps:
//! - **Fixed copy.** The text is the same for everyone with the code the only variable: no
//!   name, no account detail.
//! - **No leaks.** The phone and the code are never logged, and a provider's own error text
//!   never reaches the caller or the log: a failure is a fixed 503 `otp_unavailable`.

use std::sync::Arc;

use async_trait::async_trait;

use super::otp::{OTP_MINUTES, OtpSender};
use crate::error::AppError;

/// Why a provider did not send. Carries no provider text on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmsError {
    /// The provider could not be reached, timed out or answered with a server error.
    Unavailable,
    /// The provider refused the message (bad number, no balance, blocked sender).
    Rejected,
}

/// One text message to one number. An adapter per provider implements this.
#[async_trait]
pub trait SmsProvider: Send + Sync {
    async fn send(&self, to_e164: &str, text: &str) -> Result<(), SmsError>;
}

/// The message body for a code.
pub fn code_text(code: &str) -> String {
    format!("Your code is {code}. It expires in {OTP_MINUTES} minutes. Never share it.")
}

pub struct SmsOtpSender {
    provider: Arc<dyn SmsProvider>,
}

impl SmsOtpSender {
    pub fn new(provider: Arc<dyn SmsProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl OtpSender for SmsOtpSender {
    async fn send(&self, phone_e164: &str, code: &str) -> Result<(), AppError> {
        match self.provider.send(phone_e164, &code_text(code)).await {
            Ok(()) => Ok(()),
            Err(cause) => {
                // The cause is a fixed word: no phone, no code, no provider text.
                tracing::warn!(?cause, "sms provider did not send a code");
                Err(AppError::Unavailable("otp_unavailable"))
            }
        }
    }
}
