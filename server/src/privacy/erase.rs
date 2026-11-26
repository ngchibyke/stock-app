//! The purge: what happens to a person's data when their cooling-off ends. Every table
//! that references a user, or holds personal data, has one fate (design section 4); the
//! `privacy_inventory` test fails if a new one is not listed in [`INVENTORY`].
//!
//! The user row is never deleted. It stays as an empty shell ("Deleted user", no phone,
//! no password) so foreign keys, the ledger and the other side's history stay valid.

use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

use super::blockers::blockers;
use crate::{
    AppState, audit,
    error::AppError,
    uploads::{service::Purpose, store::Bucket},
};

/// What each table that points at a user does at erasure. The test compares this with the
/// schema, so adding a table means deciding its fate here.
///
/// `deleted`: rows go. `anonymised`: the row stays, personal columns are cleared.
/// `kept`: the row stays unlinked or until a window ends (see `retention`).
pub const INVENTORY: &[(&str, &str)] = &[
    (
        "access_code_redemptions",
        "kept: no personal data, an id link only",
    ),
    ("access_codes", "kept: staff-made"),
    ("alert_acks", "kept: staff-made"),
    ("audit_log", "kept: ids and codes only"),
    ("banned_nins", "kept while the ban stands"),
    (
        "buyer_addresses",
        "anonymised: every personal column emptied",
    ),
    ("cart_items", "deleted"),
    (
        "chat_messages",
        "kept until the chat window; author shown as Deleted user",
    ),
    ("checkouts", "kept: money record"),
    ("conversations", "kept until the chat window"),
    (
        "deletion_requests",
        "kept: the record that erasure happened",
    ),
    ("delivery_options", "deleted unless an order uses it"),
    ("disputes", "kept until the records window"),
    (
        "dispute_statements",
        "kept until the records window; photos removed",
    ),
    ("data_exports", "deleted"),
    (
        "identity_checks",
        "anonymised: name and birth year cleared; fingerprint cleared unless banned",
    ),
    ("moderation_flags", "kept until the chat window"),
    (
        "moderation_strikes",
        "kept while a ban stands, else with the chat window",
    ),
    ("notifications", "deleted"),
    (
        "orders",
        "kept until the records window; address, handover code cleared",
    ),
    ("otp_codes", "deleted"),
    ("password_reset_requests", "deleted"),
    ("payout_accounts", "anonymised"),
    ("payouts", "anonymised: account number cleared"),
    ("push_devices", "deleted"),
    ("redeem_attempts", "deleted"),
    ("reviews", "kept: no link shown; photos removed"),
    ("sessions", "deleted"),
    ("shipments", "anonymised: courier phone cleared"),
    ("sign_in_failures", "deleted"),
    ("statement_checks", "kept: staff-made"),
    (
        "store_branches",
        "anonymised for a closed store of an erased owner",
    ),
    ("store_invites", "deleted"),
    (
        "store_members",
        "ended by the users_deleted_end_memberships trigger",
    ),
    ("stores", "anonymised for a closed store of an erased owner"),
    ("uploads", "deleted with their photos"),
    ("user_moderation", "kept while a ban stands"),
    ("users", "anonymised: an empty shell"),
];

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Erased,
    /// A blocker appeared during the cooling-off: tried again on the next run.
    Deferred,
}

/// Erases one person whose request is due. Safe to run again: an erased person is a no-op.
pub async fn erase_user(state: &AppState, user: Uuid) -> Result<Outcome, AppError> {
    // Photos first: an object that cannot be deleted leaves the database untouched, so the
    // next run starts over. Deleting an object twice is not an error.
    let uploads: Vec<(String, Option<String>, String)> =
        sqlx::query_as("SELECT raw_key, key, purpose FROM uploads WHERE user_id = $1")
            .bind(user)
            .fetch_all(&state.db)
            .await
            .context("list uploads to erase")?;
    for (raw_key, key, purpose) in uploads {
        state
            .objects
            .delete(Bucket::Private, &raw_key)
            .await
            .map_err(|_| anyhow::anyhow!("photo not deleted"))?;
        if let (Some(key), Some(purpose)) = (key, Purpose::parse(&purpose)) {
            state
                .objects
                .delete(purpose.bucket(), &key)
                .await
                .map_err(|_| anyhow::anyhow!("photo not deleted"))?;
        }
    }
    erase_rows(&state.db, user).await
}

async fn erase_rows(db: &PgPool, user: Uuid) -> Result<Outcome, AppError> {
    let mut tx = db.begin().await.context("begin")?;
    let locked: Option<Option<time::OffsetDateTime>> =
        sqlx::query_scalar("SELECT erased_at FROM users WHERE id = $1 FOR UPDATE")
            .bind(user)
            .fetch_optional(&mut *tx)
            .await
            .context("lock user")?;
    match locked {
        None => return Err(AppError::NotFound),
        Some(Some(_)) => return Ok(Outcome::Erased),
        Some(None) => {}
    }
    if !blockers(&mut tx, user).await?.is_empty() {
        sqlx::query(
            "UPDATE deletion_requests SET deferred_count = deferred_count + 1 \
             WHERE user_id = $1 AND status = 'pending'",
        )
        .bind(user)
        .execute(&mut *tx)
        .await
        .context("note deferral")?;
        audit::record(
            &mut tx,
            None,
            "privacy.erasure_deferred",
            "user",
            Some(user),
            serde_json::json!({}),
        )
        .await?;
        tx.commit().await.context("commit")?;
        return Ok(Outcome::Deferred);
    }
    let phone_idx: Vec<u8> = sqlx::query_scalar("SELECT phone_idx FROM users WHERE id = $1")
        .bind(user)
        .fetch_one(&mut *tx)
        .await
        .context("read phone index")?;

    // Rows that belong to the person alone.
    for sql in [
        "DELETE FROM push_devices WHERE user_id = $1",
        "DELETE FROM notifications WHERE user_id = $1",
        "DELETE FROM sessions WHERE user_id = $1",
        "DELETE FROM otp_codes WHERE user_id = $1",
        "DELETE FROM redeem_attempts WHERE user_id = $1",
        "DELETE FROM cart_items WHERE user_id = $1",
        // An order points at its delivery option, and the option at an address, so those
        // rows stay; the address itself is emptied.
        "DELETE FROM delivery_options d WHERE d.buyer_id = $1 \
           AND NOT EXISTS (SELECT 1 FROM orders o WHERE o.delivery_option_id = d.id)",
        "UPDATE buyer_addresses SET recipient_name_enc = '\\x'::bytea, phone_enc = '\\x'::bytea, \
           address_line_enc = '\\x'::bytea, landmark_enc = NULL, postcode = NULL, \
           courier_address_code = NULL, deleted_at = COALESCE(deleted_at, now()) \
           WHERE user_id = $1",
        "DELETE FROM data_exports WHERE user_id = $1",
        // Photos: references first, then the rows (the objects are already gone).
        "UPDATE reviews SET photo_keys = '{}' WHERE buyer_id = $1",
        "UPDATE dispute_statements SET photo_keys = '{}' \
           WHERE dispute_id IN (SELECT id FROM disputes WHERE buyer_id = $1)",
        "DELETE FROM uploads WHERE user_id = $1",
        // The verified name and year of birth. The fingerprint stays only for a banned user.
        "UPDATE identity_checks SET full_name_enc = NULL, birth_year = NULL, \
           nin_idx = CASE WHEN EXISTS (SELECT 1 FROM banned_nins b WHERE b.user_id = $1) \
                          THEN nin_idx END \
           WHERE user_id = $1",
        // Orders keep the money record; the address and the code do not stay.
        "UPDATE orders SET delivery_address_enc = NULL, handover_code_enc = NULL, \
           delivery_address_purged_at = COALESCE(delivery_address_purged_at, now()) \
           WHERE buyer_id = $1",
        "UPDATE shipments SET courier_phone_enc = NULL \
           WHERE order_id IN (SELECT id FROM orders WHERE buyer_id = $1)",
        // A closed store of an erased owner: the contact details and the payout account.
        "UPDATE stores SET business_phone_enc = NULL, pickup_address_enc = NULL \
           WHERE owner_id = $1",
        "UPDATE store_branches SET business_phone_enc = NULL, pickup_address_enc = '\\x'::bytea \
           WHERE store_id IN (SELECT id FROM stores WHERE owner_id = $1)",
        "UPDATE payout_accounts SET account_number_enc = '\\x'::bytea, account_name_enc = NULL, \
           account_last4 = '0000' \
           WHERE store_id IN (SELECT id FROM stores WHERE owner_id = $1)",
        "UPDATE payouts SET account_number_enc = NULL \
           WHERE order_id IN (SELECT o.id FROM orders o JOIN stores s ON s.id = o.store_id \
                              WHERE s.owner_id = $1)",
    ] {
        sqlx::query(sql)
            .bind(user)
            .execute(&mut *tx)
            .await
            .context("erase rows")?;
    }
    // Rows keyed by the phone number rather than the user.
    for sql in [
        "DELETE FROM password_reset_requests WHERE phone_idx = $1",
        "DELETE FROM sign_in_failures WHERE phone_idx = $1",
        "DELETE FROM store_invites WHERE phone_idx = $1",
    ] {
        sqlx::query(sql)
            .bind(&phone_idx)
            .execute(&mut *tx)
            .await
            .context("erase rows by phone")?;
    }
    // The shell. The index becomes random bytes so the number can sign up again and the
    // old value can never be matched.
    sqlx::query(
        "UPDATE users SET display_name = 'Deleted user', phone_enc = '\\x'::bytea, \
            phone_idx = sha256(convert_to(gen_random_uuid()::text, 'UTF8')), \
            password_hash = '', is_seller = false, deleted_at = now(), erased_at = now() \
         WHERE id = $1",
    )
    .bind(user)
    .execute(&mut *tx)
    .await
    .context("empty the user row")?;
    sqlx::query(
        "UPDATE deletion_requests SET status = 'done', decided_at = now() \
         WHERE user_id = $1 AND status = 'pending'",
    )
    .bind(user)
    .execute(&mut *tx)
    .await
    .context("close request")?;
    audit::record(
        &mut tx,
        None,
        "privacy.erased",
        "user",
        Some(user),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Outcome::Erased)
}
