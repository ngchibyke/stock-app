use std::collections::BTreeSet;

use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// Uploads one person may make in an hour.
pub const MAX_UPLOADS_PER_HOUR: i64 = 60;

pub async fn recent_count(db: &PgPool, uploader: Uuid) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM uploads WHERE uploader_id = $1 AND created_at > now() - interval '1 hour'",
    )
    .bind(uploader)
    .fetch_one(db)
    .await
    .context("count recent uploads")?)
}

pub async fn record(
    db: &PgPool,
    key: &str,
    uploader: Uuid,
    content_type: &str,
    bytes: usize,
    width: u32,
    height: u32,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO uploads (key, uploader_id, content_type, bytes, width, height) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(key)
    .bind(uploader)
    .bind(content_type)
    .bind(bytes as i32)
    .bind(width as i32)
    .bind(height as i32)
    .execute(db)
    .await
    .context("record upload")?;
    Ok(())
}

/// Refuses a photo key the caller did not upload (422 `photo_not_yours`). Use it for every
/// key a person newly attaches to something: a product, a review, a dispute statement.
/// Keys already on a record are not re-checked by callers that only keep them.
pub async fn require_owned(db: &PgPool, user: Uuid, keys: &[String]) -> Result<(), AppError> {
    let wanted: BTreeSet<&str> = keys.iter().map(String::as_str).collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let list: Vec<&str> = wanted.iter().copied().collect();
    let owned: i64 =
        sqlx::query_scalar("SELECT count(*) FROM uploads WHERE uploader_id = $1 AND key = ANY($2)")
            .bind(user)
            .bind(&list)
            .fetch_one(db)
            .await
            .context("check photo ownership")?;
    if owned != list.len() as i64 {
        return Err(AppError::Unprocessable(
            "photo_not_yours",
            "One of those photos was not uploaded by you. Upload it again.".to_string(),
        ));
    }
    Ok(())
}
