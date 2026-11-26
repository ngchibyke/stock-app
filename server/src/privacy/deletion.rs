//! Asking for deletion, looking at the request and changing your mind. The purge itself is
//! in [`super::erase`].

use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{blockers::blockers, retention};
use crate::{
    audit,
    auth::{AuthUser, password},
    error::AppError,
};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct DeletionStatus {
    /// `none` or `pending`.
    pub state: &'static str,
    #[serde(with = "time::serde::rfc3339::option")]
    pub requested_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub due_at: Option<OffsetDateTime>,
}

impl DeletionStatus {
    fn none() -> Self {
        Self {
            state: "none",
            requested_at: None,
            due_at: None,
        }
    }
}

pub async fn status(db: &PgPool, user: Uuid) -> Result<DeletionStatus, AppError> {
    let row: Option<(OffsetDateTime, OffsetDateTime)> = sqlx::query_as(
        "SELECT requested_at, due_at FROM deletion_requests \
         WHERE user_id = $1 AND status = 'pending'",
    )
    .bind(user)
    .fetch_optional(db)
    .await
    .map_err(anyhow::Error::from)?;
    Ok(match row {
        Some((requested_at, due_at)) => DeletionStatus {
            state: "pending",
            requested_at: Some(requested_at),
            due_at: Some(due_at),
        },
        None => DeletionStatus::none(),
    })
}

/// Checks the password of the signed-in person. A wrong one is the same error as at
/// sign-in, and the work is done off the async threads.
pub async fn confirm_password(db: &PgPool, user: Uuid, given: &str) -> Result<(), AppError> {
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
        .bind(user)
        .fetch_one(db)
        .await
        .map_err(anyhow::Error::from)?;
    let given = given.to_string();
    let ok = password::blocking(move || password::verify_password(&given, &hash)).await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::InvalidCredentials)
    }
}

/// Starts the cooling-off. Asking twice is the same request.
pub async fn request(
    db: &PgPool,
    user: &AuthUser,
    given_password: &str,
) -> Result<DeletionStatus, AppError> {
    confirm_password(db, user.id, given_password).await?;
    let windows = retention::load(db).await?;
    let mut tx = db.begin().await.map_err(anyhow::Error::from)?;
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM deletion_requests WHERE user_id = $1 AND status = 'pending'",
    )
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(anyhow::Error::from)?;
    if existing.is_none() {
        let found = blockers(&mut tx, user.id).await?;
        if !found.is_empty() {
            return Err(AppError::DeletionBlocked(found));
        }
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO deletion_requests (user_id, due_at) \
             VALUES ($1, now() + make_interval(days => $2)) RETURNING id",
        )
        .bind(user.id)
        .bind(windows.deletion_cooling_off_days)
        .fetch_one(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
        // Other phones are signed out. This one stays, gated, so the person can see the
        // date and change their mind.
        sqlx::query(
            "UPDATE sessions SET revoked_at = now() \
             WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
        )
        .bind(user.id)
        .bind(user.session_id)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
        audit::record(
            &mut tx,
            Some(user.id),
            "privacy.deletion_requested",
            "deletion_request",
            Some(id),
            serde_json::json!({}),
        )
        .await?;
    }
    tx.commit().await.map_err(anyhow::Error::from)?;
    status(db, user.id).await
}

/// "Keep my account". Nothing is erased; everything works again.
pub async fn cancel(db: &PgPool, user: Uuid) -> Result<(), AppError> {
    let mut tx = db.begin().await.map_err(anyhow::Error::from)?;
    let id: Option<Uuid> = sqlx::query_scalar(
        "UPDATE deletion_requests SET status = 'cancelled', decided_at = now() \
         WHERE user_id = $1 AND status = 'pending' RETURNING id",
    )
    .bind(user)
    .fetch_optional(&mut *tx)
    .await
    .map_err(anyhow::Error::from)?;
    if let Some(id) = id {
        audit::record(
            &mut tx,
            Some(user),
            "privacy.deletion_cancelled",
            "deletion_request",
            Some(id),
            serde_json::json!({}),
        )
        .await?;
    }
    tx.commit().await.map_err(anyhow::Error::from)?;
    Ok(())
}
