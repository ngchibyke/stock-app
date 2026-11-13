//! Notifications: an insert-only outbox and inbox, and the devices pushes go to.

pub mod fake;
pub mod fcm;
pub mod kinds;
pub mod push;
pub mod reminders;
pub mod routes;
pub mod service;
pub mod worker;
