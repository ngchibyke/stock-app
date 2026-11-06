//! Sending a message.
//!
//! The order matters. Everything that refuses cheaply happens first. No
//! transaction is open during the tone check. The row and its verdict land in
//! one transaction, so a failure leaves no half-sent message, and pushes go out
//! only after the commit.

use anyhow::Context;
use sqlx::{PgConnection, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    hub::{Frame, SendOutcomeView},
    service::{self, Access, MessageView, Party, SUGGESTION_HOURS},
};
use crate::{
    AppState,
    admin::LOCK_TIMEOUT,
    checkout::service::busy_err,
    crypto::FieldCipher,
    error::AppError,
    limits::{CHAT_SENDS_PER_DAY, CHAT_SENDS_PER_MINUTE, MAX_CHAT_CHARS},
    moderation::{
        filter::Category,
        pipeline::{FlagSeverity, Verdict, judge, offline_block, reason_copy},
        strikes::{self, NewFlag, Surface, ensure_can_post, record_flag},
    },
};

/// How many of the sender's latest messages the split-digit check reads.
const RECENT_MESSAGES: i64 = 5;
/// A held row waits this long for its first retry.
const FIRST_RETRY_MINUTES: i32 = 2;

/// A message row that has not been judged yet.
pub(crate) struct Pending<'a> {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub sender_id: Uuid,
    pub order_id: Option<Uuid>,
    pub text: &'a str,
}

pub async fn send(
    state: &AppState,
    user: Uuid,
    conversation: Uuid,
    client_message_id: Uuid,
    body: &str,
) -> Result<SendOutcomeView, AppError> {
    let access = writable_access(state, user, conversation).await?;
    let text = body.trim();
    if text.is_empty() || text.chars().count() > MAX_CHAT_CHARS {
        return Err(AppError::Validation(format!(
            "a message is 1 to {MAX_CHAT_CHARS} characters"
        )));
    }
    if let Some(first) = stored(state, &access, user, client_message_id).await? {
        return Ok(first);
    }
    let mut conn = state.db.acquire().await.context("acquire")?;
    ensure_can_post(&mut conn, user).await?;
    check_rate(&mut conn, user).await?;
    let recent = recent_texts(&mut conn, &state.cipher, conversation, user).await?;
    let earlier_held = has_held(&mut conn, conversation, user).await?;
    drop(conn);

    let recent: Vec<&str> = recent.iter().map(String::as_str).collect();
    // `None` holds the message. While an earlier one is held only the offline
    // checks run, so nothing overtakes it.
    let verdict = if earlier_held {
        offline_block(text, &recent)
    } else {
        judge(state.tone.as_ref(), text, &recent, MAX_CHAT_CHARS)
            .await
            .ok()
    };

    let mut tx = begin(state).await?;
    ensure_can_post(&mut tx, user).await?;
    let inserted = insert_held(
        &mut tx,
        &state.cipher,
        &access,
        user,
        client_message_id,
        text,
    )
    .await?;
    let Some(id) = inserted else {
        tx.rollback().await.context("rollback")?;
        return first_outcome(state, &access, user, client_message_id).await;
    };
    let pending = Pending {
        id,
        conversation_id: conversation,
        sender_id: user,
        order_id: access.order_id,
        text,
    };
    let outcome = match &verdict {
        Some(verdict) => apply_in(&mut tx, &state.cipher, &pending, verdict).await?,
        None => held_outcome(),
    };
    tx.commit().await.context("commit")?;
    publish_outcome(state, &access, user, id, &outcome);
    Ok(outcome)
}

/// Sends the text the server stored for this flag, once. It is not judged
/// again: it passed the filters when it was offered.
pub async fn send_suggested(
    state: &AppState,
    user: Uuid,
    conversation: Uuid,
    flag: Uuid,
    client_message_id: Uuid,
) -> Result<SendOutcomeView, AppError> {
    let access = writable_access(state, user, conversation).await?;
    if let Some(first) = stored(state, &access, user, client_message_id).await? {
        return Ok(first);
    }
    let mut conn = state.db.acquire().await.context("acquire")?;
    ensure_can_post(&mut conn, user).await?;
    check_rate(&mut conn, user).await?;
    drop(conn);

    let mut tx = begin(state).await?;
    ensure_can_post(&mut tx, user).await?;
    let row: Option<(Option<Vec<u8>>, bool, bool)> = sqlx::query_as(
        "SELECT suggestion_enc, suggestion_used_at IS NOT NULL, \
                created_at <= now() - make_interval(hours => $4) \
         FROM moderation_flags \
         WHERE id = $1 AND user_id = $2 AND conversation_id = $3 AND surface = 'chat' \
         FOR UPDATE",
    )
    .bind(flag)
    .bind(user)
    .bind(conversation)
    .bind(SUGGESTION_HOURS)
    .fetch_optional(&mut *tx)
    .await
    .map_err(busy_err("chat_busy", "lock flag"))?;
    let (Some(encrypted), used, expired) = row.ok_or(AppError::NotFound)? else {
        return Err(AppError::NotFound);
    };
    if used {
        return Err(AppError::Conflict("suggestion_used"));
    }
    if expired {
        return Err(AppError::Conflict("suggestion_expired"));
    }
    let text = state.cipher.decrypt(&encrypted)?;
    let earlier_held = has_held(&mut tx, conversation, user).await?;
    let inserted = insert_held(
        &mut tx,
        &state.cipher,
        &access,
        user,
        client_message_id,
        &text,
    )
    .await?;
    let Some(id) = inserted else {
        tx.rollback().await.context("rollback")?;
        return first_outcome(state, &access, user, client_message_id).await;
    };
    sqlx::query("UPDATE moderation_flags SET suggestion_used_at = now() WHERE id = $1")
        .bind(flag)
        .execute(&mut *tx)
        .await
        .context("mark suggestion used")?;
    let outcome = if earlier_held {
        held_outcome()
    } else {
        let view = deliver_in(&mut tx, &state.cipher, id, conversation, &text).await?;
        delivered_outcome(view)
    };
    tx.commit().await.context("commit")?;
    publish_outcome(state, &access, user, id, &outcome);
    Ok(outcome)
}

async fn writable_access(
    state: &AppState,
    user: Uuid,
    conversation: Uuid,
) -> Result<Access, AppError> {
    let access = service::access(&state.db, user, conversation).await?;
    if !access.writable {
        return Err(AppError::Conflict("chat_closed"));
    }
    Ok(access)
}

async fn begin(state: &AppState) -> Result<Transaction<'static, Postgres>, AppError> {
    let mut tx = state.db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    Ok(tx)
}

async fn check_rate(conn: &mut PgConnection, user: Uuid) -> Result<(), AppError> {
    let (minute, day): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE created_at > now() - interval '1 minute'), count(*) \
         FROM chat_messages \
         WHERE sender_id = $1 AND created_at > now() - interval '24 hours'",
    )
    .bind(user)
    .fetch_one(&mut *conn)
    .await
    .context("count sends")?;
    if minute >= CHAT_SENDS_PER_MINUTE || day >= CHAT_SENDS_PER_DAY {
        return Err(AppError::TooManyRequests("chat_rate_limited"));
    }
    Ok(())
}

/// The sender's latest delivered messages here, oldest first, for the
/// split-digit check. Decrypted only for that check.
pub(crate) async fn recent_texts(
    conn: &mut PgConnection,
    cipher: &FieldCipher,
    conversation: Uuid,
    sender: Uuid,
) -> Result<Vec<String>, AppError> {
    let rows: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT body_enc FROM chat_messages \
         WHERE conversation_id = $1 AND sender_id = $2 AND status = 'delivered' \
           AND delivered_at > now() - interval '10 minutes' \
         ORDER BY seq DESC LIMIT $3",
    )
    .bind(conversation)
    .bind(sender)
    .bind(RECENT_MESSAGES)
    .fetch_all(&mut *conn)
    .await
    .context("read recent messages")?;
    rows.iter().rev().map(|b| cipher.decrypt(b)).collect()
}

async fn has_held(
    conn: &mut PgConnection,
    conversation: Uuid,
    sender: Uuid,
) -> Result<bool, AppError> {
    let held = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM chat_messages \
         WHERE conversation_id = $1 AND sender_id = $2 AND status = 'held')",
    )
    .bind(conversation)
    .bind(sender)
    .fetch_one(&mut *conn)
    .await
    .context("check held messages")?;
    Ok(held)
}

/// Every message starts held. `None` when this client id already has a row:
/// the insert waits for a racing send to commit, then returns nothing.
async fn insert_held(
    conn: &mut PgConnection,
    cipher: &FieldCipher,
    access: &Access,
    sender: Uuid,
    client_message_id: Uuid,
    text: &str,
) -> Result<Option<Uuid>, AppError> {
    let id = sqlx::query_scalar(
        "INSERT INTO chat_messages (conversation_id, sender_id, client_message_id, status, \
                body_enc, next_attempt_at) \
         VALUES ($1, $2, $3, 'held', $4, now() + make_interval(mins => $5)) \
         ON CONFLICT (sender_id, client_message_id) DO NOTHING RETURNING id",
    )
    .bind(access.conversation_id)
    .bind(sender)
    .bind(client_message_id)
    .bind(cipher.encrypt(text))
    .bind(FIRST_RETRY_MINUTES)
    .fetch_optional(&mut *conn)
    .await
    .map_err(busy_err("chat_busy", "insert message"))?;
    Ok(id)
}

/// Turns a held row into a delivered one: takes the conversation's next `seq`
/// under its row lock and stamps the delivery.
pub async fn deliver_in(
    conn: &mut PgConnection,
    cipher: &FieldCipher,
    message_id: Uuid,
    conversation_id: Uuid,
    text: &str,
) -> Result<MessageView, AppError> {
    let (seq, buyer_id): (i64, Uuid) = sqlx::query_as(
        "UPDATE conversations SET last_seq = last_seq + 1, last_message_at = now() \
         WHERE id = $1 RETURNING last_seq, buyer_id",
    )
    .bind(conversation_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(busy_err("chat_busy", "take sequence number"))?;
    let (sender, sent_at): (Uuid, OffsetDateTime) = sqlx::query_as(
        "UPDATE chat_messages SET status = 'delivered', seq = $2, body_enc = $3, \
                delivered_at = now(), next_attempt_at = NULL \
         WHERE id = $1 RETURNING sender_id, delivered_at",
    )
    .bind(message_id)
    .bind(seq)
    .bind(cipher.encrypt(text))
    .fetch_one(&mut *conn)
    .await
    .context("deliver message")?;
    Ok(MessageView {
        id: message_id,
        conversation_id,
        seq,
        from: if sender == buyer_id {
            Party::Buyer
        } else {
            Party::Seller
        },
        body: text.to_string(),
        sent_at,
    })
}

/// Applies a verdict to a held row. The caller owns the transaction.
pub(crate) async fn apply_in(
    conn: &mut PgConnection,
    cipher: &FieldCipher,
    pending: &Pending<'_>,
    verdict: &Verdict,
) -> Result<SendOutcomeView, AppError> {
    if *verdict == Verdict::Clean {
        let view = deliver_in(
            conn,
            cipher,
            pending.id,
            pending.conversation_id,
            pending.text,
        )
        .await?;
        return Ok(delivered_outcome(view));
    }
    let flag = record_flag(
        conn,
        cipher,
        NewFlag {
            user: pending.sender_id,
            surface: Surface::Chat,
            conversation_id: Some(pending.conversation_id),
            order_id: pending.order_id,
            verdict,
            original: pending.text,
        },
    )
    .await?;
    // The text now lives only in the encrypted flag.
    sqlx::query(
        "UPDATE chat_messages SET status = 'withheld', body_enc = NULL, flag_id = $2, \
                next_attempt_at = NULL \
         WHERE id = $1",
    )
    .bind(pending.id)
    .bind(flag.flag_id)
    .execute(&mut *conn)
    .await
    .context("withhold message")?;
    Ok(match verdict {
        Verdict::Rewrite {
            category,
            suggestion,
            ..
        } => SendOutcomeView {
            outcome: "rewrite",
            flag_id: Some(flag.flag_id),
            suggested: suggestion.clone(),
            reason: Some(reason_copy(*category)),
            ..Default::default()
        },
        Verdict::Block { category, .. } => SendOutcomeView {
            outcome: "blocked",
            flag_id: Some(flag.flag_id),
            reason: Some(reason_copy(*category)),
            strikes: flag.strike.map(|s| s.active_strikes),
            muted_until: flag.strike.and_then(|s| s.muted_until),
            ..Default::default()
        },
        Verdict::Clean => unreachable!("handled above"),
    })
}

/// Applies a late verdict to a held message, and tells both sides after commit.
/// The status is re-checked under the row lock: a row that moved on answers
/// with what already happened, and is not touched or announced again.
pub async fn apply_verdict(
    state: &AppState,
    message_id: Uuid,
    verdict: &Verdict,
) -> Result<SendOutcomeView, AppError> {
    let mut tx = begin(state).await?;
    let row: (Uuid, Uuid, Uuid, String, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT conversation_id, sender_id, client_message_id, status, body_enc \
         FROM chat_messages WHERE id = $1 FOR UPDATE",
    )
    .bind(message_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(busy_err("chat_busy", "lock message"))?;
    let (conversation_id, sender_id, client_message_id, status, body) = row;
    let access = service::access(&state.db, sender_id, conversation_id).await?;
    let (true, Some(encrypted)) = (status == "held", body) else {
        tx.rollback().await.context("rollback")?;
        return first_outcome(state, &access, sender_id, client_message_id).await;
    };
    let text = state.cipher.decrypt(&encrypted)?;
    let pending = Pending {
        id: message_id,
        conversation_id,
        sender_id,
        order_id: access.order_id,
        text: &text,
    };
    let outcome = apply_in(&mut tx, &state.cipher, &pending, verdict).await?;
    tx.commit().await.context("commit")?;
    if let Some(view) = &outcome.message {
        publish_message(state, &access, view);
    }
    state.chat_hub.publish(
        sender_id,
        Frame::Moderation {
            conversation_id,
            message_id,
            outcome: outcome.clone(),
        },
    );
    Ok(outcome)
}

pub(crate) fn held_outcome() -> SendOutcomeView {
    SendOutcomeView {
        outcome: "held",
        ..Default::default()
    }
}

fn delivered_outcome(view: MessageView) -> SendOutcomeView {
    SendOutcomeView {
        outcome: "delivered",
        message: Some(view),
        ..Default::default()
    }
}

/// To both parties, after commit.
pub fn publish_message(state: &AppState, access: &Access, view: &MessageView) {
    state
        .chat_hub
        .publish(access.buyer_id, Frame::Message(view.clone()));
    state
        .chat_hub
        .publish(access.seller_id, Frame::Message(view.clone()));
}

/// A delivered message goes to both sides. Anything else goes to the sender
/// alone: the other side must not learn a message was stopped.
pub(crate) fn publish_outcome(
    state: &AppState,
    access: &Access,
    sender: Uuid,
    message_id: Uuid,
    outcome: &SendOutcomeView,
) {
    match &outcome.message {
        Some(view) => publish_message(state, access, view),
        None => state.chat_hub.publish(
            sender,
            Frame::Moderation {
                conversation_id: access.conversation_id,
                message_id,
                outcome: outcome.clone(),
            },
        ),
    }
}

#[derive(sqlx::FromRow)]
struct StoredRow {
    id: Uuid,
    conversation_id: Uuid,
    status: String,
    seq: Option<i64>,
    body_enc: Option<Vec<u8>>,
    delivered_at: Option<OffsetDateTime>,
    flag_id: Option<Uuid>,
    severity: Option<String>,
    category: Option<String>,
    suggestion_enc: Option<Vec<u8>>,
}

/// The outcome already given for this client id, if there is one. A retry is
/// answered from here, with no tone check and no strike.
async fn stored(
    state: &AppState,
    access: &Access,
    sender: Uuid,
    client_message_id: Uuid,
) -> Result<Option<SendOutcomeView>, AppError> {
    let row: Option<StoredRow> = sqlx::query_as(
        "SELECT m.id, m.conversation_id, m.status, m.seq, m.body_enc, m.delivered_at, \
                m.flag_id, f.severity, f.category, \
                CASE WHEN f.suggestion_used_at IS NULL \
                      AND f.created_at > now() - make_interval(hours => $3) \
                     THEN f.suggestion_enc END AS suggestion_enc \
         FROM chat_messages m LEFT JOIN moderation_flags f ON f.id = m.flag_id \
         WHERE m.sender_id = $1 AND m.client_message_id = $2",
    )
    .bind(sender)
    .bind(client_message_id)
    .bind(SUGGESTION_HOURS)
    .fetch_optional(&state.db)
    .await
    .context("read stored outcome")?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.conversation_id != access.conversation_id {
        return Err(AppError::Conflict("client_message_id_reused"));
    }
    Ok(Some(stored_outcome(state, access, sender, row).await?))
}

async fn first_outcome(
    state: &AppState,
    access: &Access,
    sender: Uuid,
    client_message_id: Uuid,
) -> Result<SendOutcomeView, AppError> {
    stored(state, access, sender, client_message_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("a conflicting send left no row").into())
}

async fn stored_outcome(
    state: &AppState,
    access: &Access,
    sender: Uuid,
    row: StoredRow,
) -> Result<SendOutcomeView, AppError> {
    match row.status.as_str() {
        "delivered" => {
            let (Some(seq), Some(encrypted), Some(sent_at)) =
                (row.seq, row.body_enc, row.delivered_at)
            else {
                return Err(anyhow::anyhow!("a delivered message is incomplete").into());
            };
            Ok(delivered_outcome(MessageView {
                id: row.id,
                conversation_id: row.conversation_id,
                seq,
                from: access.party_of(sender),
                body: state.cipher.decrypt(&encrypted)?,
                sent_at,
            }))
        }
        "held" => Ok(held_outcome()),
        _ => {
            let reason = row
                .category
                .as_deref()
                .and_then(Category::parse)
                .map(reason_copy);
            if row.severity.as_deref() == Some(FlagSeverity::Mild.as_str()) {
                return Ok(SendOutcomeView {
                    outcome: "rewrite",
                    flag_id: row.flag_id,
                    suggested: row
                        .suggestion_enc
                        .map(|b| state.cipher.decrypt(&b))
                        .transpose()?,
                    reason,
                    ..Default::default()
                });
            }
            let notice = if row.severity.as_deref() == Some(FlagSeverity::Serious.as_str()) {
                let mut conn = state.db.acquire().await.context("acquire")?;
                Some(strikes::notice(&mut conn, sender).await?)
            } else {
                None
            };
            Ok(SendOutcomeView {
                outcome: "blocked",
                flag_id: row.flag_id,
                reason,
                strikes: notice.map(|n| n.active_strikes),
                muted_until: notice.and_then(|n| n.muted_until),
                ..Default::default()
            })
        }
    }
}
