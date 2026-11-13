//! Sending a notice to a device. The message is fixed copy and two ids: no
//! order, name, amount or message text ever leaves through a push.

use std::fmt;

use async_trait::async_trait;
use uuid::Uuid;

/// One push to one device.
#[derive(Clone, Copy)]
pub struct PushMessage<'a> {
    /// The device's push token. Personal data: never logged.
    pub token: &'a str,
    pub title: &'static str,
    pub body: &'static str,
    pub kind: &'static str,
    pub notification_id: Uuid,
}

impl fmt::Debug for PushMessage<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PushMessage")
            .field("token", &"[redacted]")
            .field("kind", &self.kind)
            .field("notification_id", &self.notification_id)
            .finish()
    }
}

/// Why a push did not go. No provider text is carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    /// The token is dead: disable the device.
    Unregistered,
    /// The provider refused this message for this device; retrying will not help.
    Rejected,
    /// The provider could not be reached or is overloaded; try again later.
    Unavailable,
}

#[async_trait]
pub trait PushSender: Send + Sync {
    async fn send(&self, m: &PushMessage<'_>) -> Result<(), PushError>;
}

/// Accepts every push and drops it. Outside production only, when no FCM
/// service account is configured.
pub struct NoPush;

#[async_trait]
impl PushSender for NoPush {
    async fn send(&self, _m: &PushMessage<'_>) -> Result<(), PushError> {
        Ok(())
    }
}
