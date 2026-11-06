//! Message and review moderation. Everything that judges text lives here;
//! chat and reviews only call [`pipeline`]-level functions.
//!
//! Data protection: text is personal data. It is never logged, and offline
//! checks ([`filter`]) run first so that a phone or account number is stopped
//! before anything reaches a third party.

pub mod claude;
pub mod fake;
pub mod filter;
pub mod pipeline;
pub mod strikes;
pub mod tone;
pub mod words;
