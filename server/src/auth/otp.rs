use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{Rng, RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::error::AppError;

pub const OTP_MINUTES: i32 = 10;
pub const OTP_MAX_ATTEMPTS: i32 = 5;
pub const OTP_MIN_INTERVAL_SECS: f64 = 60.0;
pub const OTP_MAX_PER_HOUR: i64 = 5;

#[async_trait]
pub trait OtpSender: Send + Sync {
    async fn send(&self, phone_e164: &str, code: &str) -> Result<(), AppError>;
}

/// Development sender. Never logs the phone or the code; when `echo` is on
/// (OTP_DEV_ECHO=1, development/test only) the code alone is written to stdout.
pub struct LogOtpSender {
    pub echo: bool,
}

#[async_trait]
impl OtpSender for LogOtpSender {
    async fn send(&self, _phone_e164: &str, code: &str) -> Result<(), AppError> {
        if self.echo {
            println!("[dev] OTP code: {code}");
        }
        Ok(())
    }
}

/// Why a code is being issued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueReason {
    /// Credentials were just set (sign-up, or a reclaim of an unverified account).
    /// Earlier codes and verification ids are always superseded, even when throttled.
    NewCredentials,
    /// Sign-in with the correct password on an unverified account (credentials unchanged).
    Resend,
}

/// The result of issuing a code. Holds secrets, so it has no `Debug`.
pub struct Issued {
    /// Returned to the client; only its SHA-256 is stored.
    pub verification_id: String,
    /// `Some` when the throttle allowed a send.
    code_to_send: Option<String>,
}

impl Issued {
    /// Hands the code to the SMS sender if this issuance was not throttled.
    /// Call only after the issuing transaction has committed.
    ///
    /// If the sender fails, the code that was never delivered is retired and stops counting
    /// toward the send throttle, so the person can ask again at once and is never handed a
    /// verification id for a code that is not on their phone.
    pub async fn deliver(
        &self,
        db: &PgPool,
        sender: &dyn OtpSender,
        user_id: Uuid,
        phone_e164: &str,
    ) -> Result<(), AppError> {
        if let Some(code) = &self.code_to_send {
            if let Err(err) = sender.send(phone_e164, code).await {
                sqlx::query(
                    "UPDATE otp_codes SET consumed_at = now(), sent_at = NULL                      WHERE verification_hash = $1",
                )
                .bind(hash_verification_id(&self.verification_id))
                .execute(db)
                .await
                .map_err(anyhow::Error::from)?;
                return Err(err);
            }
            tracing::info!(user_id = %user_id, "OTP sent to user");
        }
        Ok(())
    }
}

/// Seconds the person must wait before a new code may be sent, or `None` if one may go now.
/// One code per [`OTP_MIN_INTERVAL_SECS`], at most [`OTP_MAX_PER_HOUR`] per rolling hour.
async fn retry_after(
    conn: &mut PgConnection,
    user_id: Uuid,
    purpose: &str,
) -> Result<Option<u64>, AppError> {
    // Seconds until the newest send is old enough, and until the hourly count drops
    // below the cap (when the oldest of the capped sends leaves the window).
    let (interval_wait, hourly_wait, hourly): (Option<f64>, Option<f64>, i64) = sqlx::query_as(
        "SELECT              (SELECT $2::float8 - extract(epoch FROM now() - max(sent_at))::float8 FROM otp_codes                WHERE user_id = $1 AND purpose = $3),              (SELECT 3600::float8 - extract(epoch FROM now() - min(sent_at))::float8 FROM (                  SELECT sent_at FROM otp_codes                   WHERE user_id = $1 AND purpose = $3 AND sent_at > now() - interval '1 hour'                   ORDER BY sent_at DESC LIMIT $4) newest),              (SELECT count(*) FROM otp_codes                WHERE user_id = $1 AND purpose = $3 AND sent_at > now() - interval '1 hour')",
    )
    .bind(user_id)
    .bind(OTP_MIN_INTERVAL_SECS)
    .bind(purpose)
    .bind(OTP_MAX_PER_HOUR)
    .fetch_one(&mut *conn)
    .await
    .map_err(anyhow::Error::from)?;
    let mut wait = interval_wait.filter(|w| *w > 0.0);
    if hourly >= OTP_MAX_PER_HOUR
        && let Some(h) = hourly_wait.filter(|w| *w > 0.0)
    {
        wait = Some(wait.map_or(h, |w| w.max(h)));
    }
    Ok(wait.map(|w| w.ceil().max(1.0) as u64))
}

fn hash_code(user_id: Uuid, code: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(user_id.as_bytes());
    h.update(code.as_bytes());
    h.finalize().to_vec()
}

fn hash_verification_id(verification_id: &str) -> Vec<u8> {
    Sha256::digest(verification_id.as_bytes()).to_vec()
}

fn new_verification_id() -> String {
    let mut raw = [0u8; 32];
    OsRng.fill_bytes(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Retires every outstanding code (and with it every verification id) for the user.
pub async fn invalidate_otps(conn: &mut PgConnection, user_id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE otp_codes SET consumed_at = now() WHERE user_id = $1 AND consumed_at IS NULL",
    )
    .bind(user_id)
    .execute(conn)
    .await
    .map_err(anyhow::Error::from)?;
    Ok(())
}

/// Issues a verification inside the caller's transaction (which must also hold any
/// credential change it is bound to). Deliver the code with [`Issued::deliver`] after commit.
///
/// A verification id is only ever returned for a code that will be sent, so the throttle
/// answers with [`AppError::OtpThrottled`] instead of an id for an unsent code:
/// - `NewCredentials`: when throttled, fails. The caller's transaction rolls back, so the
///   credentials do not change and earlier codes keep working for the old credentials.
/// - `Resend`: when throttled, the live sent code is re-bound to a fresh verification id
///   (the code already on the phone stays valid, the previous id stops working). With no
///   live sent code, fails.
pub async fn issue_otp(
    conn: &mut PgConnection,
    user_id: Uuid,
    reason: IssueReason,
) -> Result<Issued, AppError> {
    // Serialise issuance per user so concurrent calls cannot slip past the throttle.
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(anyhow::Error::from)?;
    let wait = retry_after(conn, user_id, "verify_phone").await?;
    let verification_id = new_verification_id();

    if let Some(retry_after_secs) = wait {
        if reason == IssueReason::Resend {
            let rebound: Option<Uuid> = sqlx::query_scalar(
                "UPDATE otp_codes SET verification_hash = $2 WHERE id = (                      SELECT id FROM otp_codes                      WHERE user_id = $1 AND purpose = 'verify_phone' AND consumed_at IS NULL                        AND sent_at IS NOT NULL AND expires_at > now()                      ORDER BY created_at DESC LIMIT 1)                  RETURNING id",
            )
            .bind(user_id)
            .bind(hash_verification_id(&verification_id))
            .fetch_optional(&mut *conn)
            .await
            .map_err(anyhow::Error::from)?;
            if rebound.is_some() {
                return Ok(Issued {
                    verification_id,
                    code_to_send: None,
                });
            }
        }
        return Err(AppError::OtpThrottled { retry_after_secs });
    }

    invalidate_otps(conn, user_id).await?;
    let code = format!("{:06}", OsRng.gen_range(0..1_000_000u32));
    sqlx::query(
        "INSERT INTO otp_codes (user_id, code_hash, verification_hash, expires_at, sent_at)          VALUES ($1, $2, $3, now() + make_interval(mins => $4), now())",
    )
    .bind(user_id)
    .bind(hash_code(user_id, &code))
    .bind(hash_verification_id(&verification_id))
    .bind(OTP_MINUTES)
    .execute(&mut *conn)
    .await
    .map_err(anyhow::Error::from)?;
    Ok(Issued {
        verification_id,
        code_to_send: Some(code),
    })
}

/// Issues a password-reset code. Unlike [`issue_otp`] it never supersedes a live code when
/// throttled: spamming the route must not kill a code the account holder is about to use.
/// When the throttle blocks a send, nothing is stored and the returned id matches nothing,
/// so the caller's answer looks the same either way.
pub async fn issue_reset_otp(conn: &mut PgConnection, user_id: Uuid) -> Result<Issued, AppError> {
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(anyhow::Error::from)?;
    let (recent, hourly): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE sent_at > now() - make_interval(secs => $2)),                 count(*) FILTER (WHERE sent_at > now() - interval '1 hour')          FROM otp_codes WHERE user_id = $1 AND purpose = 'password_reset'",
    )
    .bind(user_id)
    .bind(OTP_MIN_INTERVAL_SECS)
    .fetch_one(&mut *conn)
    .await
    .map_err(anyhow::Error::from)?;
    let verification_id = new_verification_id();
    if recent > 0 || hourly >= OTP_MAX_PER_HOUR {
        return Ok(Issued {
            verification_id,
            code_to_send: None,
        });
    }
    invalidate_otps(conn, user_id).await?;
    let code = format!("{:06}", OsRng.gen_range(0..1_000_000u32));
    sqlx::query(
        "INSERT INTO otp_codes (user_id, code_hash, verification_hash, expires_at, sent_at, purpose) \n         VALUES ($1, $2, $3, now() + make_interval(mins => $4), now(), 'password_reset')",
    )
    .bind(user_id)
    .bind(hash_code(user_id, &code))
    .bind(hash_verification_id(&verification_id))
    .bind(OTP_MINUTES)
    .execute(&mut *conn)
    .await
    .map_err(anyhow::Error::from)?;
    Ok(Issued {
        verification_id,
        code_to_send: Some(code),
    })
}

fn invalid() -> AppError {
    AppError::Unprocessable("otp_invalid", "That code is not correct".into())
}

fn locked() -> AppError {
    AppError::Unprocessable(
        "otp_locked",
        "Too many wrong codes. Request a new code".into(),
    )
}

/// Checks `code` against the live, sent OTP bound to `verification_id`. On success the
/// code is consumed, the phone is marked verified and the user's id is returned.
/// Every guess counts toward the 5-attempt cap.
pub async fn verify_otp(db: &PgPool, verification_id: &str, code: &str) -> Result<Uuid, AppError> {
    check_code(db, verification_id, code, "verify_phone").await
}

/// Checks a password-reset code the same way. The code is consumed, but nothing else
/// changes: the caller sets the password. A code for another purpose never matches.
pub async fn verify_reset_otp(
    db: &PgPool,
    verification_id: &str,
    code: &str,
) -> Result<Uuid, AppError> {
    check_code(db, verification_id, code, "password_reset").await
}

async fn check_code(
    db: &PgPool,
    verification_id: &str,
    code: &str,
    purpose: &str,
) -> Result<Uuid, AppError> {
    let row: Option<(Uuid, Uuid, Vec<u8>, i32, bool)> = sqlx::query_as(
        "SELECT id, user_id, code_hash, attempts, expires_at < now() FROM otp_codes \
         WHERE verification_hash = $1 AND consumed_at IS NULL AND sent_at IS NOT NULL \n           AND purpose = $2",
    )
    .bind(hash_verification_id(verification_id))
    .bind(purpose)
    .fetch_optional(db)
    .await
    .map_err(anyhow::Error::from)?;
    let Some((otp_id, user_id, code_hash, attempts, expired)) = row else {
        return Err(invalid());
    };
    if attempts >= OTP_MAX_ATTEMPTS {
        return Err(locked());
    }
    if expired {
        return Err(AppError::Unprocessable(
            "otp_expired",
            "That code has expired. Request a new code".into(),
        ));
    }
    // Atomically count this guess so a concurrent burst cannot exceed the cap.
    let counted: Option<i32> = sqlx::query_scalar(
        "UPDATE otp_codes SET attempts = attempts + 1 WHERE id = $1 AND attempts < $2 RETURNING attempts",
    )
    .bind(otp_id)
    .bind(OTP_MAX_ATTEMPTS)
    .fetch_optional(db)
    .await
    .map_err(anyhow::Error::from)?;
    if counted.is_none() {
        return Err(locked());
    }
    if !ct_eq(&hash_code(user_id, code), &code_hash) {
        return Err(invalid());
    }

    let mut tx = db.begin().await.map_err(anyhow::Error::from)?;
    // Same lock as credential changes: a concurrent reclaim either commits first (and this
    // code is already superseded) or waits until the phone is verified (and then fails).
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    let consumed = sqlx::query(
        "UPDATE otp_codes SET consumed_at = now() WHERE id = $1 AND consumed_at IS NULL",
    )
    .bind(otp_id)
    .execute(&mut *tx)
    .await
    .map_err(anyhow::Error::from)?
    .rows_affected();
    if consumed == 0 {
        return Err(invalid());
    }
    if purpose == "verify_phone" {
        sqlx::query(
            "UPDATE users SET phone_verified_at = COALESCE(phone_verified_at, now()) WHERE id = $1",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(anyhow::Error::from)?;
    }
    tx.commit().await.map_err(anyhow::Error::from)?;
    Ok(user_id)
}
