//! The self-delivery handover code.
//!
//! A self-delivery order gets a 6-digit code (from the OS CSPRNG) when the
//! seller sets off with it (`picked_up -> out_for_delivery`); it is stored
//! encrypted with `FieldCipher` (`orders.handover_code_enc`) because the buyer
//! is shown it while the order is out for delivery. The seller enters it on
//! handover; entering it is the buyer's confirmation and releases the money.
//!
//! [`verify`] guards it against guessing: after 5 wrong entries, entry is
//! locked for an hour (409 `handover_locked`, even for the right code) and the
//! counter starts again. Every attempt is audited as `order.handover_attempt`
//! with `{ok, locked}` only: the code (entered or expected) is never audited,
//! logged or returned. Courier orders never have a code.

use anyhow::Context;
use rand::{Rng, rngs::OsRng};
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{audit, crypto::FieldCipher, error::AppError};

/// Wrong entries that start a lock.
const MAX_FAILURES: i32 = 5;
/// Digits in a code.
const CODE_LEN: usize = 6;

/// A fresh handover code: 6 decimal digits, uniformly from the OS CSPRNG.
pub fn generate() -> String {
    format!("{:06}", OsRng.gen_range(0..1_000_000u32))
}

/// Equality whose running time does not depend on where the inputs differ.
/// (The length is not secret: every code is 6 digits.)
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(diff) == 0
}

/// Checks `code` against the order's handover code.
///
/// Runs inside the caller's transaction, under the order lock (it takes the
/// row lock itself as well). The caller must **commit even when this fails**
/// with `handover_code_wrong` or `handover_locked`, so the failure counter,
/// the lock and the audit row stick.
///
/// - locked -> 409 `handover_locked` (the counter is not touched);
/// - no code on the order (not out for delivery) -> 409 `invalid_order_state`;
/// - not 6 digits (blank, a slip) -> 422 `handover_code_wrong`, not an attempt:
///   it cannot be the code, so it is neither counted nor audited;
/// - mismatch -> `handover_failures + 1`; the 5th sets `handover_locked_until
///   = now() + 1h` and resets the counter; 422 `handover_code_wrong`;
/// - match -> resets the counter; Ok.
pub async fn verify(
    conn: &mut PgConnection,
    cipher: &FieldCipher,
    actor: Option<Uuid>,
    order_id: Uuid,
    code: &str,
) -> Result<(), AppError> {
    let (enc, locked): (Option<Vec<u8>>, bool) = sqlx::query_as(
        "SELECT handover_code_enc, COALESCE(handover_locked_until > now(), false) \
         FROM orders WHERE id = $1 FOR UPDATE",
    )
    .bind(order_id)
    .fetch_optional(&mut *conn)
    .await
    .context("lock order for handover")?
    .ok_or(AppError::NotFound)?;
    if locked {
        attempt(conn, actor, order_id, false, true).await?;
        return Err(AppError::ConflictDetail(
            "handover_locked",
            "Too many wrong codes. Try again in an hour.".into(),
        ));
    }
    let Some(enc) = enc else {
        return Err(AppError::Conflict("invalid_order_state"));
    };
    if !well_formed(code) {
        return Err(wrong_code());
    }
    let expected = cipher.decrypt(&enc)?;
    if constant_time_eq(expected.as_bytes(), code.as_bytes()) {
        sqlx::query("UPDATE orders SET handover_failures = 0 WHERE id = $1")
            .bind(order_id)
            .execute(&mut *conn)
            .await
            .context("reset handover failures")?;
        attempt(conn, actor, order_id, true, false).await?;
        return Ok(());
    }
    let locked_now: bool = sqlx::query_scalar(
        "UPDATE orders SET \
             handover_failures = CASE WHEN handover_failures + 1 >= $2 THEN 0 \
                                      ELSE handover_failures + 1 END, \
             handover_locked_until = CASE WHEN handover_failures + 1 >= $2 \
                                          THEN now() + interval '1 hour' \
                                          ELSE handover_locked_until END \
         WHERE id = $1 \
         RETURNING handover_locked_until IS NOT NULL AND handover_locked_until > now()",
    )
    .bind(order_id)
    .bind(MAX_FAILURES)
    .fetch_one(&mut *conn)
    .await
    .context("count handover failure")?;
    attempt(conn, actor, order_id, false, locked_now).await?;
    Err(wrong_code())
}

fn wrong_code() -> AppError {
    AppError::Unprocessable(
        "handover_code_wrong",
        "That code is not right. Ask the buyer for the code shown in their order.".into(),
    )
}

/// Could `code` be a handover code at all?
fn well_formed(code: &str) -> bool {
    code.len() == CODE_LEN && code.bytes().all(|b| b.is_ascii_digit())
}

/// Audits one attempt. Never the code.
async fn attempt(
    conn: &mut PgConnection,
    actor: Option<Uuid>,
    order_id: Uuid,
    ok: bool,
    locked: bool,
) -> Result<(), AppError> {
    audit::record(
        conn,
        actor,
        "order.handover_attempt",
        "order",
        Some(order_id),
        json!({ "ok": ok, "locked": locked }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..1000 {
            let c = generate();
            assert_eq!(c.len(), 6, "{c}");
            assert!(c.bytes().all(|b| b.is_ascii_digit()), "{c}");
        }
    }

    #[test]
    fn codes_vary() {
        let codes: std::collections::HashSet<String> = (0..50).map(|_| generate()).collect();
        assert!(codes.len() > 40);
    }

    #[test]
    fn only_six_digits_are_well_formed() {
        assert!(well_formed("012345"));
        for bad in ["", "12345", "1234567", "12345a", " 12345", "١٢٣٤٥٦"] {
            assert!(!well_formed(bad), "{bad}");
        }
    }

    #[test]
    fn compare() {
        assert!(constant_time_eq(b"123456", b"123456"));
        assert!(!constant_time_eq(b"123456", b"123457"));
        assert!(!constant_time_eq(b"123456", b"12345"));
        assert!(!constant_time_eq(b"123456", b""));
    }
}
