//! Password reset by phone code.
//!
//! The request route answers the same way whether or not the phone has an account, so it
//! cannot be used to find out who is registered. A code is sent only to a verified,
//! undeleted account. Confirming sets the new password, ends every session of the user and
//! clears the sign-in lockout.

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    otp::{OTP_MIN_INTERVAL_SECS, issue_reset_otp, verify_reset_otp},
    password::{blocking, hash_password, validate_password},
};
use crate::{
    AppState, audit, crypto::blind_index, error::AppError, extract::AppJson,
    phone::normalize_ng_phone,
};

/// Reset requests allowed per phone in a rolling hour, whether or not it has an account.
pub const MAX_RESET_REQUESTS_PER_HOUR: i64 = 10;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/password-reset", post(request_reset))
        .route("/auth/password-reset/confirm", post(confirm_reset))
}

#[derive(Deserialize)]
struct ResetRequest {
    phone: String,
}

#[derive(Deserialize)]
struct ResetConfirm {
    verification_id: String,
    code: String,
    new_password: String,
}

/// Counts this request against the phone's hourly cap (serialised per phone, so a parallel
/// burst cannot overshoot it). Applies equally to phones with and without an account.
async fn reserve_request(db: &PgPool, phone_idx: &[u8]) -> Result<(), AppError> {
    let mut tx = db.begin().await.map_err(anyhow::Error::from)?;
    let lock_key = i64::from_be_bytes(phone_idx[..8].try_into().expect("blind index is 32 bytes"));
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    sqlx::query("DELETE FROM password_reset_requests WHERE at <= now() - interval '1 hour'")
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    let recent: i64 =
        sqlx::query_scalar("SELECT count(*) FROM password_reset_requests WHERE phone_idx = $1")
            .bind(phone_idx)
            .fetch_one(&mut *tx)
            .await
            .map_err(anyhow::Error::from)?;
    if recent >= MAX_RESET_REQUESTS_PER_HOUR {
        return Err(AppError::TooManyRequests("too_many_attempts"));
    }
    sqlx::query("INSERT INTO password_reset_requests (phone_idx) VALUES ($1)")
        .bind(phone_idx)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    tx.commit().await.map_err(anyhow::Error::from)?;
    Ok(())
}

/// A well-formed answer that matches no code: what an unknown phone gets.
fn decoy_id() -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use rand::{RngCore, rngs::OsRng};
    let mut raw = [0u8; 32];
    OsRng.fill_bytes(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

async fn request_reset(
    State(state): State<AppState>,
    AppJson(req): AppJson<ResetRequest>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let phone = normalize_ng_phone(&req.phone)?;
    let phone_idx = blind_index(&state.index_key, &phone);
    reserve_request(&state.db, &phone_idx).await?;

    let user: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users \
         WHERE phone_idx = $1 AND deleted_at IS NULL AND phone_verified_at IS NOT NULL",
    )
    .bind(&phone_idx)
    .fetch_optional(&state.db)
    .await
    .map_err(anyhow::Error::from)?;

    let verification_id = match user {
        Some(user_id) => {
            let mut tx = state.db.begin().await.map_err(anyhow::Error::from)?;
            let issued = issue_reset_otp(&mut tx, user_id).await?;
            tx.commit().await.map_err(anyhow::Error::from)?;
            // A sender failure must not turn into a different answer: that would show
            // which phones have accounts. It is logged (without the phone) instead.
            if issued
                .deliver(state.otp.as_ref(), user_id, &phone)
                .await
                .is_err()
            {
                tracing::warn!(user_id = %user_id, "password reset code could not be sent");
            }
            issued.verification_id
        }
        None => decoy_id(),
    };
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "verification_id": verification_id,
            "resend_after_secs": OTP_MIN_INTERVAL_SECS as u64,
        })),
    ))
}

fn code_invalid() -> AppError {
    AppError::Unprocessable(
        "code_invalid",
        "That code is not correct or has expired".into(),
    )
}

async fn confirm_reset(
    State(state): State<AppState>,
    AppJson(req): AppJson<ResetConfirm>,
) -> Result<StatusCode, AppError> {
    // Checked first so a refused password does not use up the code.
    validate_password(&req.new_password)?;
    let password = req.new_password;
    let hash = blocking(move || hash_password(&password)).await??;

    // Every way a code can be bad (wrong, expired, locked, unknown, another purpose) reads alike.
    let user_id =
        match verify_reset_otp(&state.db, req.verification_id.trim(), req.code.trim()).await {
            Ok(id) => id,
            Err(AppError::Unprocessable(..)) => return Err(code_invalid()),
            Err(other) => return Err(other),
        };

    let mut tx = state.db.begin().await.map_err(anyhow::Error::from)?;
    let phone_idx: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT phone_idx FROM users \
         WHERE id = $1 AND deleted_at IS NULL AND phone_verified_at IS NOT NULL \
         FOR NO KEY UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(anyhow::Error::from)?;
    let Some(phone_idx) = phone_idx else {
        return Err(code_invalid());
    };
    sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(user_id)
        .bind(&hash)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    sqlx::query("DELETE FROM sign_in_failures WHERE phone_idx = $1")
        .bind(&phone_idx)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    audit::record(
        &mut tx,
        Some(user_id),
        "auth.password_reset",
        "user",
        Some(user_id),
        json!({}),
    )
    .await?;
    tx.commit().await.map_err(anyhow::Error::from)?;
    Ok(StatusCode::NO_CONTENT)
}
