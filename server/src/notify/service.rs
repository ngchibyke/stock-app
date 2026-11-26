//! Raising notices (insert-only). Notices hold ids and a kind, never text.

use anyhow::Context;
use sqlx::PgConnection;
use uuid::Uuid;

use super::kinds::Kind;
use crate::{
    error::AppError,
    stores::access::{covers_branch_sql, is_member_sql},
};

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

/// Which part of a store's team a notice goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// The owner, the managers, and staff who may act on the order's branch.
    Order,
    /// The owner and the managers (disputes, reviews).
    Managers,
    /// The owner alone (payout account, store status).
    OwnerOnly,
}

/// The users of the order's store to be told, read from the database now. Removed
/// members, and staff tied to a closed branch, are never included in `Order`.
async fn team_recipients(
    conn: &mut PgConnection,
    order_id: Uuid,
    audience: Audience,
) -> Result<Vec<Uuid>, AppError> {
    let rule = match audience {
        Audience::OwnerOnly => "u.id = s.owner_id".to_string(),
        Audience::Order => covers_branch_sql("u.id", "o.branch_id"),
        Audience::Managers => format!(
            "({} AND (u.id = s.owner_id OR EXISTS (SELECT 1 FROM store_members sm \
             WHERE sm.store_id = s.id AND sm.user_id = u.id AND sm.removed_at IS NULL \
             AND sm.role = 'manager')))",
            is_member_sql("u.id")
        ),
    };
    let sql = format!(
        "SELECT u.id FROM orders o JOIN stores s ON s.id = o.store_id \
         JOIN users u ON u.id = s.owner_id OR u.id IN (SELECT sm.user_id FROM store_members sm \
             WHERE sm.store_id = s.id AND sm.removed_at IS NULL) \
         WHERE o.id = $1 AND {rule}"
    );
    let ids = sqlx::query_scalar(&sql)
        .bind(order_id)
        .fetch_all(conn)
        .await
        .context("load team recipients")?;
    Ok(ids)
}

/// Raises an order notice for the part of the order's store team named by
/// `audience`, in the caller's transaction.
pub async fn tell_team(
    conn: &mut PgConnection,
    kind: Kind,
    order_id: Uuid,
    audience: Audience,
) -> Result<(), AppError> {
    for user in team_recipients(conn, order_id, audience).await? {
        enqueue(conn, Notice::order(user, kind, order_id)).await?;
    }
    Ok(())
}

/// As [`tell_team`], for a notice about a dispute.
pub async fn tell_team_dispute(
    conn: &mut PgConnection,
    kind: Kind,
    order_id: Uuid,
    dispute_id: Uuid,
    audience: Audience,
) -> Result<(), AppError> {
    for user in team_recipients(conn, order_id, audience).await? {
        enqueue(conn, Notice::dispute(user, kind, dispute_id)).await?;
    }
    Ok(())
}

/// Raises a dispute notice for the order's buyer and the store's owner and
/// managers, in the caller's transaction.
pub async fn tell_both(
    conn: &mut PgConnection,
    kind: Kind,
    order_id: Uuid,
    dispute_id: Uuid,
) -> Result<(), AppError> {
    let (buyer, _) = order_parties(conn, order_id).await?;
    enqueue(conn, Notice::dispute(buyer, kind, dispute_id)).await?;
    tell_team_dispute(conn, kind, order_id, dispute_id, Audience::Managers).await
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
