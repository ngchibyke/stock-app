//! Photo uploads (plan 7, S2): checking and cleaning an image, storing it, and
//! remembering who uploaded it so only they can attach it to something.

pub mod process;
pub mod routes;
pub mod service;
pub mod store;

pub use store::{MemoryStore, ObjectStore, S3Config, S3Store};
