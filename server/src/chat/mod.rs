//! Buyer and seller chat. Every message passes
//! [`moderation`](crate::moderation) before the other side can see it.

pub mod hub;
pub mod routes;
pub mod service;
