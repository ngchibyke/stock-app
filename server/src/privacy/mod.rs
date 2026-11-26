//! Part 8: data rights and retention. A person can ask for their data and for deletion
//! (a 30-day cooling-off, then a purge), and the server removes personal data on windows
//! the Owner sets.
//!
//! Data protection: nothing here logs a name, phone, address or message. Audit rows carry
//! ids and fixed codes only. The export file is encrypted under the field key and only its
//! owner can download it.

pub mod blockers;
pub mod deletion;
pub mod erase;
pub mod export;
pub mod jobs;
pub mod retention;
pub mod routes;
