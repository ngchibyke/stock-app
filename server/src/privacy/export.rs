//! A person's copy of their data. Asked for in the app with the password; a job builds one
//! JSON file, stored encrypted under the field key, available for [`LIFETIME_DAYS`] days and
//! only to the person it belongs to.

use anyhow::Context;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::deletion::confirm_password;
use crate::{AppState, audit, crypto::FieldCipher, error::AppError, escrow::jobs::Stats};

pub const BUILD_EXPORTS: &str = "build_exports";
pub const LIFETIME_DAYS: i32 = 7;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ExportStatus {
    /// `none`, `pending`, `ready`, `failed` or `expired`.
    pub state: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub requested_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

pub async fn status(db: &PgPool, user: Uuid) -> Result<ExportStatus, AppError> {
    let row: Option<(String, OffsetDateTime, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT status, requested_at, expires_at FROM data_exports \
         WHERE user_id = $1 ORDER BY requested_at DESC LIMIT 1",
    )
    .bind(user)
    .fetch_optional(db)
    .await
    .context("read export")?;
    Ok(match row {
        Some((state, requested_at, expires_at)) => ExportStatus {
            state,
            requested_at: Some(requested_at),
            expires_at,
        },
        None => ExportStatus {
            state: "none".into(),
            requested_at: None,
            expires_at: None,
        },
    })
}

/// Asks for an export. A live one (pending or ready) is returned as it is; otherwise one a
/// day.
pub async fn request(
    db: &PgPool,
    user: Uuid,
    given_password: &str,
) -> Result<ExportStatus, AppError> {
    confirm_password(db, user, given_password).await?;
    let mut tx = db.begin().await.context("begin")?;
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await
        .context("lock user")?;
    let live: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM data_exports \
                        WHERE user_id = $1 AND status IN ('pending', 'ready'))",
    )
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .context("find live export")?;
    if !live {
        let recent: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM data_exports \
                            WHERE user_id = $1 AND requested_at > now() - interval '24 hours')",
        )
        .bind(user)
        .fetch_one(&mut *tx)
        .await
        .context("find recent export")?;
        if recent {
            return Err(AppError::TooManyRequests("export_rate_limited"));
        }
        let id: Uuid =
            sqlx::query_scalar("INSERT INTO data_exports (user_id) VALUES ($1) RETURNING id")
                .bind(user)
                .fetch_one(&mut *tx)
                .await
                .context("insert export")?;
        audit::record(
            &mut tx,
            Some(user),
            "privacy.export_requested",
            "data_export",
            Some(id),
            json!({}),
        )
        .await?;
    }
    tx.commit().await.context("commit")?;
    status(db, user).await
}

/// The file, for its owner only, while it is ready.
pub async fn file(state: &AppState, user: Uuid) -> Result<Vec<u8>, AppError> {
    let row: Option<(Uuid, Vec<u8>)> = sqlx::query_as(
        "SELECT id, body_enc FROM data_exports \
         WHERE user_id = $1 AND status = 'ready' AND expires_at > now()",
    )
    .bind(user)
    .fetch_optional(&state.db)
    .await
    .context("read export file")?;
    let (id, body) = row.ok_or(AppError::NotFound)?;
    let plain = state.cipher.decrypt(&body)?;
    let mut conn = state.db.acquire().await.context("acquire")?;
    audit::record(
        &mut conn,
        Some(user),
        "privacy.export_downloaded",
        "data_export",
        Some(id),
        json!({}),
    )
    .await?;
    Ok(plain.into_bytes())
}

/// Job `build_exports`: builds each pending export.
pub async fn run(state: &AppState, cancel: &CancellationToken) -> Result<Stats, AppError> {
    let pending: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, user_id FROM data_exports WHERE status = 'pending' \
         ORDER BY requested_at LIMIT 20",
    )
    .fetch_all(&state.db)
    .await
    .context("find exports to build")?;
    let mut stats = Stats::default();
    for (id, user) in pending {
        if cancel.is_cancelled() {
            break;
        }
        match build(&state.db, &state.cipher, user).await {
            Ok(doc) => {
                let body = state.cipher.encrypt(&doc.to_string());
                sqlx::query(
                    "UPDATE data_exports SET status = 'ready', body_enc = $2, ready_at = now(), \
                        expires_at = now() + make_interval(days => $3) WHERE id = $1",
                )
                .bind(id)
                .bind(body)
                .bind(LIFETIME_DAYS)
                .execute(&state.db)
                .await
                .context("store export")?;
                stats.done += 1;
            }
            Err(e) => {
                tracing::warn!(job = BUILD_EXPORTS, error = %e, "export not built");
                sqlx::query("UPDATE data_exports SET status = 'failed' WHERE id = $1")
                    .bind(id)
                    .execute(&state.db)
                    .await
                    .context("mark export failed")?;
                stats.failed += 1;
            }
        }
    }
    Ok(stats)
}

fn ts(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn opt_ts(t: Option<OffsetDateTime>) -> Value {
    t.map(|t| json!(ts(t))).unwrap_or(Value::Null)
}

fn dec(cipher: &FieldCipher, blob: &Option<Vec<u8>>) -> Value {
    blob.as_deref()
        .and_then(|b| cipher.decrypt(b).ok())
        .map(Value::from)
        .unwrap_or(Value::Null)
}

/// The person's own data: their account, and what they wrote or were sent. Other people's
/// data is left out (the other party's statement in a dispute, the identity name, the NIN).
pub async fn build(db: &PgPool, cipher: &FieldCipher, user: Uuid) -> Result<Value, AppError> {
    let (name, phone_enc, created, role, seller): (String, Vec<u8>, OffsetDateTime, String, bool) =
        sqlx::query_as(
            "SELECT display_name, phone_enc, created_at, role::text, is_seller FROM users WHERE id = $1",
        )
        .bind(user)
        .fetch_one(db)
        .await
        .context("export profile")?;

    let addresses: Vec<(
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        String,
        Option<String>,
        OffsetDateTime,
    )> = sqlx::query_as(
        "SELECT recipient_name_enc, phone_enc, address_line_enc, landmark_enc, \
                    area_council::text, postcode, created_at \
             FROM buyer_addresses WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .context("export addresses")?;
    let addresses: Vec<Value> = addresses
        .into_iter()
        .map(|(n, p, a, l, area, post, at)| {
            json!({
                "recipient_name": dec(cipher, &n), "phone": dec(cipher, &p),
                "address": dec(cipher, &a), "landmark": dec(cipher, &l),
                "area_council": area, "postcode": post, "created_at": ts(at),
            })
        })
        .collect();

    let orders: Vec<(
        Uuid,
        String,
        i64,
        i64,
        OffsetDateTime,
        Option<OffsetDateTime>,
        Option<Vec<u8>>,
    )> = sqlx::query_as(
        "SELECT id, status::text, items_total_kobo, delivery_fee_kobo, created_at, paid_at, \
                    delivery_address_enc FROM orders WHERE buyer_id = $1 ORDER BY created_at",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .context("export orders")?;
    let mut order_docs = Vec::new();
    for (id, status, items, fee, created_at, paid_at, addr) in orders {
        let lines: Vec<(String, i32, i64)> = sqlx::query_as(
            "SELECT product_name, quantity, unit_price_kobo FROM order_items WHERE order_id = $1",
        )
        .bind(id)
        .fetch_all(db)
        .await
        .context("export order lines")?;
        order_docs.push(json!({
            "id": id, "status": status, "items_total_kobo": items, "delivery_fee_kobo": fee,
            "created_at": ts(created_at), "paid_at": opt_ts(paid_at),
            "delivery_address": dec(cipher, &addr),
            "lines": lines.into_iter().map(|(n, q, p)| json!({"name": n, "quantity": q, "unit_price_kobo": p})).collect::<Vec<_>>(),
        }));
    }

    let messages: Vec<(Uuid, Option<i64>, bool, Option<Vec<u8>>, OffsetDateTime)> = sqlx::query_as(
        "SELECT c.id, m.seq, m.sender_id = $1, m.body_enc, m.created_at \
         FROM chat_messages m JOIN conversations c ON c.id = m.conversation_id \
         WHERE m.status = 'delivered' AND (c.buyer_id = $1 OR m.sender_id = $1) \
         ORDER BY c.id, m.seq",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .context("export chats")?;
    let chats: Vec<Value> = messages
        .into_iter()
        .map(|(conv, seq, mine, body, at)| {
            json!({"conversation": conv, "seq": seq, "from_you": mine,
                   "text": dec(cipher, &body), "sent_at": ts(at)})
        })
        .collect();

    let reviews: Vec<(Uuid, i16, Option<String>, OffsetDateTime)> = sqlx::query_as(
        "SELECT order_id, stars, body, created_at FROM reviews WHERE buyer_id = $1 ORDER BY created_at",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .context("export reviews")?;
    let reviews: Vec<Value> = reviews
        .into_iter()
        .map(|(o, s, b, at)| json!({"order": o, "stars": s, "text": b, "created_at": ts(at)}))
        .collect();

    let disputes: Vec<(Uuid, Uuid, String, String, OffsetDateTime, Option<Vec<u8>>)> =
        sqlx::query_as(
            "SELECT d.id, d.order_id, d.reason, d.status, d.opened_at, s.body_enc \
         FROM disputes d LEFT JOIN dispute_statements s \
              ON s.dispute_id = d.id AND s.party = 'buyer' \
         WHERE d.buyer_id = $1 ORDER BY d.opened_at",
        )
        .bind(user)
        .fetch_all(db)
        .await
        .context("export disputes")?;
    let disputes: Vec<Value> = disputes
        .into_iter()
        .map(|(id, order, reason, status, at, body)| {
            json!({"id": id, "order": order, "reason": reason, "status": status,
                   "opened_at": ts(at), "your_statement": dec(cipher, &body)})
        })
        .collect();

    let notices: Vec<(String, OffsetDateTime, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT kind, created_at, read_at FROM notifications WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .context("export notices")?;
    let notices: Vec<Value> = notices
        .into_iter()
        .map(|(k, c, r)| json!({"kind": k, "created_at": ts(c), "read_at": opt_ts(r)}))
        .collect();

    let uploads: Vec<(Uuid, String, String, OffsetDateTime)> = sqlx::query_as(
        "SELECT id, purpose, status, created_at FROM uploads WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .context("export uploads")?;
    let uploads: Vec<Value> = uploads
        .into_iter()
        .map(|(id, p, s, c)| json!({"id": id, "purpose": p, "status": s, "created_at": ts(c)}))
        .collect();

    let members: Vec<(Uuid, String, OffsetDateTime, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT store_id, role::text, created_at, removed_at FROM store_members \
         WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .context("export memberships")?;
    let members: Vec<Value> = members
        .into_iter()
        .map(|(s, r, c, x)| json!({"store": s, "role": r, "since": ts(c), "left": opt_ts(x)}))
        .collect();

    let identity: Vec<(String, OffsetDateTime)> = sqlx::query_as(
        "SELECT status, started_at FROM identity_checks WHERE user_id = $1 ORDER BY started_at",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .context("export identity")?;
    let identity: Vec<Value> = identity
        .into_iter()
        .map(|(s, at)| json!({"status": s, "at": ts(at)}))
        .collect();

    Ok(json!({
        "generated_at": ts(OffsetDateTime::now_utc()),
        "profile": {
            "display_name": name, "phone": cipher.decrypt(&phone_enc).ok(),
            "created_at": ts(created), "role": role, "is_seller": seller,
        },
        "addresses": addresses, "orders": order_docs, "chats": chats, "reviews": reviews,
        "disputes": disputes, "notices": notices, "uploads": uploads,
        "store_memberships": members, "identity_checks": identity,
    }))
}
