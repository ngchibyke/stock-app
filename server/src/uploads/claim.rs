//! Attaching finished uploads to a resource.
//!
//! A photo key typed into a request is only a string. Before it is stored on a
//! product, a store, a review or a dispute statement, it must be a `ready` upload
//! that this user made for this purpose and that nothing else has claimed. The
//! check and the claim are one step under a row lock, inside the caller's
//! transaction, so two requests cannot both take the same photo and a failed
//! request leaves its keys free.
//!
//! Every way a key can be unusable (unknown, still pending, someone else's, made
//! for another purpose) answers with the same message, so keys cannot be probed
//! to learn what exists.

use std::collections::HashSet;

use anyhow::Context;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use super::service::Purpose;
use crate::error::AppError;

/// What a photo ends up attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Product,
    Store,
    Review,
    Dispute,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Product => "product",
            Kind::Store => "store",
            Kind::Review => "review",
            Kind::Dispute => "dispute",
        }
    }
}

fn not_yours() -> AppError {
    AppError::Unprocessable(
        "photo_not_yours",
        "One of the photos is not available; upload it again".into(),
    )
}

/// Claims `keys` for `attach`. Call it after the resource's own lock, in the
/// same transaction as the write that stores the keys. Keys already attached to
/// this same resource are fine (a retry, or a dispute reply reusing a photo).
pub async fn claim(
    conn: &mut PgConnection,
    user: Uuid,
    purpose: Purpose,
    keys: &[String],
    attach: (Kind, Uuid),
) -> Result<(), AppError> {
    let wanted: HashSet<&str> = keys.iter().map(String::as_str).collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let list: Vec<&str> = wanted.iter().copied().collect();
    let rows = sqlx::query(
        "SELECT id, user_id, purpose, status, attached_kind, attached_id \
         FROM uploads WHERE key = ANY($1) ORDER BY id FOR UPDATE",
    )
    .bind(&list)
    .fetch_all(&mut *conn)
    .await
    .context("lock uploads")?;
    if rows.len() != wanted.len() {
        return Err(not_yours());
    }
    let mut taken = false;
    for row in &rows {
        let owner: Uuid = row.get("user_id");
        let made_for: String = row.get("purpose");
        let status: String = row.get("status");
        if owner != user || made_for != purpose.as_str() || status != "ready" {
            return Err(not_yours());
        }
        let kind: Option<String> = row.get("attached_kind");
        let id: Option<Uuid> = row.get("attached_id");
        if let (Some(kind), Some(id)) = (kind, id)
            && (kind != attach.0.as_str() || id != attach.1)
        {
            taken = true;
        }
    }
    if taken {
        return Err(AppError::Conflict("photo_in_use"));
    }
    sqlx::query("UPDATE uploads SET attached_kind = $2, attached_id = $3 WHERE key = ANY($1)")
        .bind(&list)
        .bind(attach.0.as_str())
        .bind(attach.1)
        .execute(&mut *conn)
        .await
        .context("attach uploads")?;
    Ok(())
}

/// The keys in `new` that `current` did not already hold: only those are checked,
/// so rows written before ownership checks existed keep working.
pub fn added(current: &[String], new: &[String]) -> Vec<String> {
    new.iter()
        .filter(|k| !current.contains(k))
        .cloned()
        .collect()
}

/// Public addresses for `keys`, in order. Empty when no public store is configured.
pub fn public_urls(objects: &dyn super::store::ObjectStore, keys: &[String]) -> Vec<String> {
    keys.iter().filter_map(|k| objects.public_url(k)).collect()
}
