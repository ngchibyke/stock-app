//! Raising notices (insert-only). Notices hold ids and a kind, never text.

use anyhow::Context;
use sqlx::PgConnection;
use uuid::Uuid;

use super::kinds::Kind;
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct Notice {
    pub user: Uuid,
    pub kind: Kind,
    pub order_id: Option<Uuid>,
    pub dispute_id: Option<Uuid>,
    pub conversation_id: Option<Uuid>,
    pub dedupe_key: String,
}

impl Notice {
    pub fn order(user: Uuid, kind: Kind, order_id: Uuid) -> Notice {
        Notice {
            user,
            kind,
            order_id: Some(order_id),
            dispute_id: None,
            conversation_id: None,
            dedupe_key: format!("{}:{order_id}", kind.as_str()),
        }
    }

    /// A notice about the user's own account, with no subject.
    pub fn user(user: Uuid, kind: Kind, dedupe_key: String) -> Notice {
        Notice {
            user,
            kind,
            order_id: None,
            dispute_id: None,
            conversation_id: None,
            dedupe_key,
        }
    }

    pub fn dispute(user: Uuid, kind: Kind, dispute_id: Uuid) -> Notice {
        Notice {
            user,
            kind,
            order_id: None,
            dispute_id: Some(dispute_id),
            conversation_id: None,
            dedupe_key: format!("{}:{dispute_id}", kind.as_str()),
        }
    }
}

/// Inserts a notice; the same `(user, dedupe_key)` twice leaves one row.
pub async fn enqueue(conn: &mut PgConnection, n: Notice) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO notifications \
            (user_id, kind, order_id, dispute_id, conversation_id, dedupe_key) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (user_id, dedupe_key) DO NOTHING",
    )
    .bind(n.user)
    .bind(n.kind.as_str())
    .bind(n.order_id)
    .bind(n.dispute_id)
    .bind(n.conversation_id)
    .bind(&n.dedupe_key)
    .execute(conn)
    .await
    .context("enqueue notification")?;
    Ok(())
}

/// Marks the user's unread `chat_message` notices for one conversation read.
pub async fn mark_conversation_read(
    conn: &mut PgConnection,
    user: Uuid,
    conversation_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE notifications SET read_at = now() \
         WHERE user_id = $1 AND conversation_id = $2 AND kind = 'chat_message' \
           AND read_at IS NULL",
    )
    .bind(user)
    .bind(conversation_id)
    .execute(conn)
    .await
    .context("mark conversation notices read")?;
    Ok(())
}

/// The order's buyer and its seller (the owner of the order's store).
pub async fn order_parties(
    conn: &mut PgConnection,
    order_id: Uuid,
) -> Result<(Uuid, Uuid), AppError> {
    let parties = sqlx::query_as(
        "SELECT o.buyer_id, s.owner_id FROM orders o JOIN stores s ON s.id = o.store_id \
         WHERE o.id = $1",
    )
    .bind(order_id)
    .fetch_one(conn)
    .await
    .context("load order parties")?;
    Ok(parties)
}

/// Raises a dispute notice for both sides of the dispute's order, in the
/// caller's transaction.
pub async fn tell_both(
    conn: &mut PgConnection,
    kind: Kind,
    order_id: Uuid,
    dispute_id: Uuid,
) -> Result<(), AppError> {
    let (buyer, seller) = order_parties(conn, order_id).await?;
    enqueue(conn, Notice::dispute(buyer, kind, dispute_id)).await?;
    enqueue(conn, Notice::dispute(seller, kind, dispute_id)).await
}

/// Raises an order notice for the order's buyer, in the caller's transaction.
pub async fn tell_buyer(
    conn: &mut PgConnection,
    kind: Kind,
    order_id: Uuid,
) -> Result<(), AppError> {
    let (buyer, _) = order_parties(conn, order_id).await?;
    enqueue(conn, Notice::order(buyer, kind, order_id)).await
}

/// Raises an order notice for the order's seller, in the caller's transaction.
pub async fn tell_seller(
    conn: &mut PgConnection,
    kind: Kind,
    order_id: Uuid,
) -> Result<(), AppError> {
    let (_, seller) = order_parties(conn, order_id).await?;
    enqueue(conn, Notice::order(seller, kind, order_id)).await
}
