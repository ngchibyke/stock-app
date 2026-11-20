//! Forgotten-password recovery by phone code.
//!
//! The request answers the same way whether or not the phone has an account, so it
//! cannot be used to find out who is registered. Confirming sets the new password,
//! ends every session of the account and clears its sign-in lockout.

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    otp::{IssueReason, OTP_MIN_INTERVAL_SECS, Purpose, issue_otp, verify_code},
    password::{blocking, hash_password, validate_password},
};
use crate::{
    AppState, audit, crypto::blind_index, error::AppError, extract::AppJson,
    phone::normalize_ng_phone,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/password-reset", post(request_reset))
        .route("/auth/password-reset/confirm", post(confirm_reset))
}

#[derive(Deserialize)]
struct Request {
    phone: String,
}

#[derive(Deserialize)]
struct Confirm {
    verification_id: String,
    code: String,
    new_password: String,
}

/// A well-formed id that is bound to nothing, for phones with no account.
fn decoy_verification_id() -> String {
    let mut raw = [0u8; 32];
    OsRng.fill_bytes(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

fn body(verification_id: String) -> Json<Value> {
    Json(json!({
        "verification_id": verification_id,
        "resend_after_secs": OTP_MIN_INTERVAL_SECS as u64,
    }))
}

async fn request_reset(
    State(state): State<AppState>,
    AppJson(req): AppJson<Request>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let phone = normalize_ng_phone(&req.phone)?;
    let phone_idx = blind_index(&state.index_key, &phone);
    // Only a verified, live account gets a code. Everyone else gets the same answer.
    let user: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM users \
         WHERE phone_idx = $1 AND deleted_at IS NULL AND phone_verified_at IS NOT NULL",
    )
    .bind(&phone_idx)
    .fetch_optional(&state.db)
    .await
    .map_err(anyhow::Error::from)?;
    let Some(user_id) = user else {
        return Ok((StatusCode::ACCEPTED, body(decoy_verification_id())));
    };

    let mut tx = state.db.begin().await.map_err(anyhow::Error::from)?;
    // A throttled request rebinds the live code to a new id (the code already on the
    // phone stays valid); it never hands out an id for a code that does not exist.
    let issued = issue_otp(
        &mut tx,
        user_id,
        IssueReason::Resend,
        Purpose::PasswordReset,
    )
    .await?;
    tx.commit().await.map_err(anyhow::Error::from)?;
    issued.deliver(state.otp.as_ref(), user_id, &phone).await?;
    Ok((StatusCode::ACCEPTED, body(issued.verification_id)))
}

fn code_invalid() -> AppError {
    AppError::Unprocessable(
        "code_invalid",
        "That code is not correct or has expired. Request a new code".into(),
    )
}

async fn confirm_reset(
    State(state): State<AppState>,
    AppJson(req): AppJson<Confirm>,
) -> Result<StatusCode, AppError> {
    // Checked before the code is spent, so a weak password costs the person nothing.
    validate_password(&req.new_password)?;
    let user_id = verify_code(
        &state.db,
        req.verification_id.trim(),
        req.code.trim(),
        Purpose::PasswordReset,
    )
    .await
    .map_err(|e| match e {
        AppError::Unprocessable(..) => code_invalid(),
        other => other,
    })?;

    let password = req.new_password;
    let hash = blocking(move || hash_password(&password)).await??;

    let mut tx = state.db.begin().await.map_err(anyhow::Error::from)?;
    let phone_idx: Option<Vec<u8>> = sqlx::query_scalar(
        "UPDATE users SET password_hash = $2 \
         WHERE id = $1 AND deleted_at IS NULL RETURNING phone_idx",
    )
    .bind(user_id)
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(anyhow::Error::from)?;
    let Some(phone_idx) = phone_idx else {
        return Err(code_invalid());
    };
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
