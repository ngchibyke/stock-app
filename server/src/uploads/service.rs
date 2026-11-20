//! Asking for an upload slot and finalizing an upload.
//!
//! **Why the raw file goes to the private bucket.** A phone uploads the original,
//! which can carry a GPS position and other metadata. It is read only by the
//! server, re-encoded, and the clean copy is written to its real home (public for
//! product, store and review photos; private for dispute evidence). The original
//! is deleted. So the public bucket never holds an unprocessed photo, not even
//! for a moment.
//!
//! Object-store calls happen outside any database transaction.

use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    image_check::{self, Rejected},
    store::{Bucket, StoreError},
};
use crate::{
    AppState, audit,
    auth::AuthUser,
    error::AppError,
    limits::{MAX_UPLOAD_BYTES, UPLOAD_URL_TTL_SECS, UPLOADS_PER_HOUR},
    stores::routes::require_seller,
};

/// What a photo is for. The purpose decides who may upload it and where it ends up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Product,
    StoreLogo,
    StoreBanner,
    Review,
    Dispute,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Product => "product",
            Purpose::StoreLogo => "store_logo",
            Purpose::StoreBanner => "store_banner",
            Purpose::Review => "review",
            Purpose::Dispute => "dispute",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Purpose::Product,
            Purpose::StoreLogo,
            Purpose::StoreBanner,
            Purpose::Review,
            Purpose::Dispute,
        ]
        .into_iter()
        .find(|p| p.as_str() == s)
    }

    /// Only a seller puts photos on a product or a store.
    fn seller_only(self) -> bool {
        matches!(
            self,
            Purpose::Product | Purpose::StoreLogo | Purpose::StoreBanner
        )
    }

    /// Where the finished photo lives. Evidence is never public.
    pub fn bucket(self) -> Bucket {
        match self {
            Purpose::Dispute => Bucket::Private,
            _ => Bucket::Public,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UploadRequest {
    pub purpose: String,
    pub content_type: String,
    pub bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct Created {
    pub upload_id: Uuid,
    pub url: String,
    /// What the phone must send with the `PUT`, exactly.
    pub headers: serde_json::Map<String, serde_json::Value>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
pub struct Finalized {
    pub upload_id: Uuid,
    pub key: String,
}

fn unavailable() -> AppError {
    AppError::Unavailable("uploads_unavailable")
}

fn rejected(r: Rejected) -> AppError {
    AppError::Unprocessable("photo_rejected", r.message().into())
}

/// A slot for one upload. Writes the row, then signs the URL (no network).
pub async fn create(
    state: &AppState,
    user: &AuthUser,
    req: UploadRequest,
) -> Result<Created, AppError> {
    if !state.objects.available() {
        return Err(unavailable());
    }
    let purpose = Purpose::parse(&req.purpose).ok_or_else(|| {
        AppError::Validation(
            "purpose must be product, store_logo, store_banner, review or dispute".into(),
        )
    })?;
    if purpose.seller_only() {
        require_seller(user)?;
    }
    if !["image/jpeg", "image/png", "image/webp"].contains(&req.content_type.as_str()) {
        return Err(AppError::Validation(
            "content_type must be image/jpeg, image/png or image/webp".into(),
        ));
    }
    if !(1..=MAX_UPLOAD_BYTES).contains(&req.bytes) {
        return Err(AppError::Validation(format!(
            "bytes must be between 1 and {MAX_UPLOAD_BYTES}"
        )));
    }

    let mut tx = state.db.begin().await.context("begin")?;
    // One request at a time per person, so the hourly cap cannot be raced past.
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(user.id)
        .fetch_one(&mut *tx)
        .await
        .context("lock user")?;
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM uploads WHERE user_id = $1 AND created_at > now() - interval '1 hour'",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .context("count recent uploads")?;
    if recent >= UPLOADS_PER_HOUR {
        return Err(AppError::TooManyRequests("upload_rate_limited"));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO uploads (user_id, purpose, raw_key, declared_type, declared_bytes) \
         VALUES ($1, $2, 'raw/' || gen_random_uuid()::text, $3, $4) RETURNING id",
    )
    .bind(user.id)
    .bind(purpose.as_str())
    .bind(&req.content_type)
    .bind(req.bytes as i32)
    .fetch_one(&mut *tx)
    .await
    .context("insert upload")?;
    let raw_key: String = sqlx::query_scalar("SELECT raw_key FROM uploads WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .context("read raw key")?;
    audit::record(
        &mut tx,
        Some(user.id),
        "upload.created",
        "upload",
        Some(id),
        json!({ "purpose": purpose.as_str(), "bytes": req.bytes }),
    )
    .await?;
    tx.commit().await.context("commit")?;

    let ttl = Duration::from_secs(UPLOAD_URL_TTL_SECS);
    let signed = state
        .objects
        .presign_put(
            Bucket::Private,
            &raw_key,
            &req.content_type,
            req.bytes as u64,
            ttl,
        )
        .map_err(|_| unavailable())?;
    let headers = signed
        .headers
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    Ok(Created {
        upload_id: id,
        url: signed.url,
        headers,
        expires_at: OffsetDateTime::now_utc() + ttl,
    })
}

struct Slot {
    purpose: Purpose,
    raw_key: String,
    key: Option<String>,
    declared_type: String,
    declared_bytes: i64,
    status: String,
    failure: Option<String>,
}

async fn load(db: &PgPool, id: Uuid, user: Uuid) -> Result<Slot, AppError> {
    let row = sqlx::query(
        "SELECT purpose, raw_key, key, declared_type, declared_bytes::bigint AS declared_bytes, \
                status, failure \
         FROM uploads WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user)
    .fetch_optional(db)
    .await
    .context("load upload")?
    .ok_or(AppError::NotFound)?;
    let purpose: String = row.get("purpose");
    Ok(Slot {
        purpose: Purpose::parse(&purpose).ok_or_else(|| anyhow::anyhow!("unknown purpose"))?,
        raw_key: row.get("raw_key"),
        key: row.get("key"),
        declared_type: row.get("declared_type"),
        declared_bytes: row.get("declared_bytes"),
        status: row.get("status"),
        failure: row.get("failure"),
    })
}

/// Checks the uploaded file and stores the clean copy. Safe to call again.
pub async fn finalize(state: &AppState, user: Uuid, id: Uuid) -> Result<Finalized, AppError> {
    let slot = load(&state.db, id, user).await?;
    if let Some(done) = already_decided(&slot, id) {
        return done;
    }
    if !state.objects.available() {
        return Err(unavailable());
    }

    let raw = match state
        .objects
        .get(Bucket::Private, &slot.raw_key, slot.declared_bytes as usize)
        .await
    {
        Ok(bytes) => bytes,
        Err(StoreError::NotFound) => {
            // Another finalize may have finished and cleared the raw file meanwhile.
            let now = load(&state.db, id, user).await?;
            return already_decided(&now, id).unwrap_or_else(|| {
                Err(AppError::Unprocessable(
                    "upload_missing",
                    "The photo has not been uploaded yet".into(),
                ))
            });
        }
        Err(StoreError::TooLarge) => return fail(state, id, user, &slot, Rejected::TooLarge).await,
        Err(StoreError::Unavailable) => return Err(unavailable()),
    };
    if raw.len() as i64 != slot.declared_bytes {
        return fail(state, id, user, &slot, Rejected::SizeMismatch).await;
    }

    // Decoding is CPU work on untrusted input: off the async threads, and bounded in time.
    let content_type = slot.declared_type.clone();
    let checked = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || image_check::reencode(&raw, &content_type)),
    )
    .await;
    let jpeg = match checked {
        Ok(Ok(Ok(bytes))) => bytes,
        Ok(Ok(Err(reason))) => return fail(state, id, user, &slot, reason).await,
        _ => return fail(state, id, user, &slot, Rejected::Corrupt).await,
    };

    // The same key for every attempt, so two finalizes at once write one object.
    let key = format!("u/{}/{id}.jpg", slot.purpose.as_str());
    let out_bytes = jpeg.len();
    state
        .objects
        .put(slot.purpose.bucket(), &key, jpeg, "image/jpeg")
        .await
        .map_err(|_| unavailable())?;

    let mut tx = state.db.begin().await.context("begin")?;
    let won = sqlx::query(
        "UPDATE uploads SET status = 'ready', key = $2, finalized_at = now() \
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .bind(&key)
    .execute(&mut *tx)
    .await
    .context("mark upload ready")?
    .rows_affected()
        == 1;
    if won {
        audit::record(
            &mut tx,
            Some(user),
            "upload.finalized",
            "upload",
            Some(id),
            json!({ "purpose": slot.purpose.as_str(), "bytes": out_bytes }),
        )
        .await?;
    }
    tx.commit().await.context("commit")?;
    delete_raw(state, &slot.raw_key).await;
    Ok(Finalized { upload_id: id, key })
}

/// The answer for an upload already settled one way or the other.
fn already_decided(slot: &Slot, id: Uuid) -> Option<Result<Finalized, AppError>> {
    match slot.status.as_str() {
        "ready" => slot
            .key
            .clone()
            .map(|key| Ok(Finalized { upload_id: id, key })),
        "failed" => Some(Err(rejected(
            slot.failure
                .as_deref()
                .and_then(Rejected::parse)
                .unwrap_or(Rejected::Corrupt),
        ))),
        _ => None,
    }
}

/// Marks the upload failed (once), deletes the raw file, and answers 422.
async fn fail(
    state: &AppState,
    id: Uuid,
    user: Uuid,
    slot: &Slot,
    reason: Rejected,
) -> Result<Finalized, AppError> {
    let mut tx = state.db.begin().await.context("begin")?;
    let changed = sqlx::query(
        "UPDATE uploads SET status = 'failed', failure = $2 WHERE id = $1 AND status = 'pending'",
    )
    .bind(id)
    .bind(reason.code())
    .execute(&mut *tx)
    .await
    .context("mark upload failed")?
    .rows_affected();
    if changed == 1 {
        audit::record(
            &mut tx,
            Some(user),
            "upload.rejected",
            "upload",
            Some(id),
            json!({ "purpose": slot.purpose.as_str(), "reason": reason.code() }),
        )
        .await?;
    }
    tx.commit().await.context("commit")?;
    delete_raw(state, &slot.raw_key).await;
    Err(rejected(reason))
}

/// Deleting the raw file is tidying: a failure is logged and the purge job tries again.
async fn delete_raw(state: &AppState, raw_key: &str) {
    if state
        .objects
        .delete(Bucket::Private, raw_key)
        .await
        .is_err()
    {
        tracing::warn!("raw upload not deleted; the purge job retries");
    }
}
