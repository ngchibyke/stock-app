//! Conversations between one buyer and one store, tied to a product or an order.
//!
//! **Access.** The buyer, and the store's side: the owner, its managers, its
//! store-wide staff, and staff tied to a branch for that branch's order chats
//! (and for product chats, which belong to no branch). Everyone else, platform
//! staff included, gets 404: platform staff read a chat through a flag, or when
//! a dispute opens, never by browsing. Membership is read on every request.
//!
//! **What each side sees.** The seller side sees "Buyer", the buyer sees the
//! store's name. Neither sees the other's name, id or phone. The store's team
//! also sees which member wrote each of the store's messages; the buyer never does.

use anyhow::Context;
use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    admin::LOCK_TIMEOUT,
    cart::service::{BUYABLE, own_store_item},
    checkout::service::busy_err,
    crypto::FieldCipher,
    error::AppError,
    limits::NEW_CONVERSATIONS_PER_DAY,
    moderation::{filter::Category, pipeline::reason_copy},
    orders::views::Cursor,
    stores::access::{covers_branch_sql, is_member_sql},
};

/// What a seller sees instead of a buyer's name.
pub const BUYER_LABEL: &str = "Buyer";
/// How long a rewrite suggestion can be sent.
pub const SUGGESTION_HOURS: i32 = 1;
const DEFAULT_PAGE: i64 = 30;
const MAX_PAGE: i64 = 100;
const DEFAULT_MESSAGES: i64 = 50;
const MAX_MESSAGES: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Party {
    Buyer,
    Seller,
}

/// The caller's place in a conversation, and both sides' ids for pushing.
#[derive(Debug, Clone)]
pub struct Access {
    pub conversation_id: Uuid,
    pub buyer_id: Uuid,
    /// The store's owner.
    pub seller_id: Uuid,
    /// Everyone on the store's side who may see this conversation (the owner
    /// included): who live frames and notices go to.
    pub team: Vec<Uuid>,
    pub store_id: Uuid,
    pub order_id: Option<Uuid>,
    pub party: Party,
    pub writable: bool,
}

impl Access {
    /// The side `sender` is on.
    pub fn party_of(&self, sender: Uuid) -> Party {
        if sender == self.buyer_id {
            Party::Buyer
        } else {
            Party::Seller
        }
    }
}

/// SQL boolean: the user bound at `user_param` is on the store's side of the
/// conversation `c` (of store `s`, about order `ord`, if any): the owner, a manager,
/// store-wide staff, or staff tied to a branch, who see product chats and the chats of
/// their own open branch's orders.
pub fn seller_side_sql(user_param: &str) -> String {
    format!(
        "(s.owner_id = {user_param} OR EXISTS (SELECT 1 FROM store_members sm \
         WHERE sm.store_id = s.id AND sm.user_id = {user_param} AND sm.removed_at IS NULL \
           AND (sm.role = 'manager' OR sm.branch_id IS NULL OR c.order_id IS NULL \
                OR (sm.branch_id = ord.branch_id AND EXISTS ( \
                      SELECT 1 FROM store_branches sb \
                      WHERE sb.id = sm.branch_id AND sb.closed_at IS NULL)))))"
    )
}

/// Everyone on the store's side of a conversation about `store` and `order`.
pub async fn seller_side_users(
    conn: &mut sqlx::PgConnection,
    store: Uuid,
    order: Option<Uuid>,
) -> Result<Vec<Uuid>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT s.owner_id FROM stores s WHERE s.id = $1 \
         UNION \
         SELECT sm.user_id FROM store_members sm \
         LEFT JOIN orders ord ON ord.id = $2 \
         WHERE sm.store_id = $1 AND sm.removed_at IS NULL \
           AND (sm.role = 'manager' OR sm.branch_id IS NULL OR $2::uuid IS NULL \
                OR (sm.branch_id = ord.branch_id AND EXISTS ( \
                      SELECT 1 FROM store_branches sb \
                      WHERE sb.id = sm.branch_id AND sb.closed_at IS NULL)))",
    )
    .bind(store)
    .bind(order)
    .fetch_all(conn)
    .await
    .context("read the store's side of a conversation")?)
}

/// `writable` turns false once the store cannot trade or either account is
/// gone. The history stays readable.
pub async fn access(db: &PgPool, user: Uuid, conversation: Uuid) -> Result<Access, AppError> {
    let row: Option<(Uuid, Uuid, Uuid, Option<Uuid>, bool)> = sqlx::query_as(&format!(
        "SELECT c.buyer_id, s.owner_id, c.store_id, c.order_id, \
                (s.status = 'active' AND s.deleted_at IS NULL \
                 AND b.deleted_at IS NULL AND o.deleted_at IS NULL) \
         FROM conversations c \
         JOIN stores s ON s.id = c.store_id \
         JOIN users b ON b.id = c.buyer_id \
         JOIN users o ON o.id = s.owner_id \
         LEFT JOIN orders ord ON ord.id = c.order_id \
         WHERE c.id = $1 AND (c.buyer_id = $2 OR {})",
        seller_side_sql("$2")
    ))
    .bind(conversation)
    .bind(user)
    .fetch_optional(db)
    .await
    .context("read conversation access")?;
    let (buyer_id, seller_id, store_id, order_id, writable) = row.ok_or(AppError::NotFound)?;
    let mut conn = db.acquire().await.context("acquire")?;
    let team = seller_side_users(&mut conn, store_id, order_id).await?;
    Ok(Access {
        conversation_id: conversation,
        buyer_id,
        seller_id,
        team,
        store_id,
        order_id,
        party: if user == buyer_id {
            Party::Buyer
        } else {
            Party::Seller
        },
        writable,
    })
}

pub enum Subject {
    Product(Uuid),
    Order(Uuid),
}

/// Opens the conversation, or returns the one that already exists.
pub async fn open(db: &PgPool, user: Uuid, subject: Subject) -> Result<ConversationView, AppError> {
    let id = match subject {
        Subject::Product(product) => open_product(db, user, product).await?,
        Subject::Order(order) => open_order(db, user, order).await?,
    };
    view_one(db, user, id).await
}

async fn open_product(db: &PgPool, user: Uuid, product: Uuid) -> Result<Uuid, AppError> {
    let row: Option<(Uuid, bool, bool)> = sqlx::query_as(&format!(
        "SELECT p.store_id, ({}), ({BUYABLE}) \
         FROM products p JOIN stores s ON s.id = p.store_id WHERE p.id = $1",
        is_member_sql("$2")
    ))
    .bind(product)
    .bind(user)
    .fetch_optional(db)
    .await
    .context("read product for chat")?;
    let (store, own_store, buyable) = row.ok_or(AppError::NotFound)?;
    if own_store {
        return Err(own_store_item());
    }
    if !buyable {
        return Err(AppError::NotFound);
    }

    let mut tx = db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    // Serialises one buyer's opens, so the daily cap cannot be raced.
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await
        .map_err(busy_err("chat_busy", "lock user"))?;
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM conversations WHERE buyer_id = $1 AND product_id = $2")
            .bind(user)
            .bind(product)
            .fetch_optional(&mut *tx)
            .await
            .context("find conversation")?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let started: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM conversations \
         WHERE buyer_id = $1 AND product_id IS NOT NULL \
           AND created_at > now() - interval '24 hours'",
    )
    .bind(user)
    .fetch_one(&mut *tx)
    .await
    .context("count new conversations")?;
    if started >= NEW_CONVERSATIONS_PER_DAY {
        return Err(AppError::TooManyRequests("chat_rate_limited"));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO conversations (buyer_id, store_id, product_id) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(user)
    .bind(store)
    .bind(product)
    .fetch_one(&mut *tx)
    .await
    .context("insert conversation")?;
    tx.commit().await.context("commit")?;
    Ok(id)
}

/// Order chats have no cap: each one needs a real, paid order.
async fn open_order(db: &PgPool, user: Uuid, order: Uuid) -> Result<Uuid, AppError> {
    let row: Option<(Uuid, Uuid, bool)> = sqlx::query_as(&format!(
        "SELECT o.buyer_id, o.store_id, ({}) \
         FROM orders o JOIN stores s ON s.id = o.store_id \
         WHERE o.id = $1 AND o.paid_at IS NOT NULL",
        covers_branch_sql("$2", "o.branch_id")
    ))
    .bind(order)
    .bind(user)
    .fetch_optional(db)
    .await
    .context("read order for chat")?;
    // The buyer, or the store's side that may act on the order's branch.
    let (buyer, store, on_store_side) = row.ok_or(AppError::NotFound)?;
    if user != buyer && !on_store_side {
        return Err(AppError::NotFound);
    }
    sqlx::query(
        "INSERT INTO conversations (buyer_id, store_id, order_id) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (order_id) WHERE order_id IS NOT NULL DO NOTHING",
    )
    .bind(buyer)
    .bind(store)
    .bind(order)
    .execute(db)
    .await
    .context("insert conversation")?;
    let id = sqlx::query_scalar("SELECT id FROM conversations WHERE order_id = $1")
        .bind(order)
        .fetch_one(db)
        .await
        .context("read conversation")?;
    Ok(id)
}

#[derive(Debug, Serialize)]
pub struct ConversationView {
    pub id: Uuid,
    /// The other side's label: the store's name, or "Buyer".
    pub with: String,
    pub you: Party,
    pub store_name: String,
    pub store_id: Uuid,
    pub product_id: Option<Uuid>,
    pub order_id: Option<Uuid>,
    /// The branch an order chat is about; only the store's side is told.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    pub writable: bool,
    pub unread: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub last_message_at: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct ConversationRow {
    id: Uuid,
    buyer_id: Uuid,
    store_id: Uuid,
    branch_name: Option<String>,
    store_name: String,
    product_id: Option<Uuid>,
    order_id: Option<Uuid>,
    writable: bool,
    unread: i64,
    last_message_at: OffsetDateTime,
}

impl ConversationRow {
    fn view(self, user: Uuid) -> ConversationView {
        let you = if user == self.buyer_id {
            Party::Buyer
        } else {
            Party::Seller
        };
        ConversationView {
            id: self.id,
            with: match you {
                Party::Buyer => self.store_name.clone(),
                Party::Seller => BUYER_LABEL.to_string(),
            },
            you,
            store_name: self.store_name,
            store_id: self.store_id,
            branch_name: match you {
                Party::Buyer => None,
                Party::Seller => self.branch_name,
            },
            product_id: self.product_id,
            order_id: self.order_id,
            writable: self.writable,
            unread: self.unread,
            last_message_at: self.last_message_at,
        }
    }
}

/// `$1` is the viewer.
const LIST_SELECT: &str = "SELECT c.id, c.buyer_id, c.store_id, s.name AS store_name, c.product_id, \
        c.order_id, c.last_message_at, \
        (SELECT sb.name FROM store_branches sb WHERE sb.id = ord.branch_id) AS branch_name, \
        (s.status = 'active' AND s.deleted_at IS NULL \
         AND b.deleted_at IS NULL AND o.deleted_at IS NULL) AS writable, \
        (SELECT count(*) FROM chat_messages m \
         WHERE m.conversation_id = c.id AND m.status = 'delivered' \
           AND (CASE WHEN c.buyer_id = $1 THEN m.sender_id <> c.buyer_id \
                     ELSE m.sender_id = c.buyer_id END) \
           AND m.seq > CASE WHEN c.buyer_id = $1 THEN c.buyer_read_seq \
                            ELSE c.seller_read_seq END) AS unread \
     FROM conversations c \
     JOIN stores s ON s.id = c.store_id \
     JOIN users b ON b.id = c.buyer_id \
     JOIN users o ON o.id = s.owner_id \
     LEFT JOIN orders ord ON ord.id = c.order_id ";

async fn view_one(db: &PgPool, user: Uuid, id: Uuid) -> Result<ConversationView, AppError> {
    let row: ConversationRow = sqlx::query_as(&format!("{LIST_SELECT} WHERE c.id = $2"))
        .bind(user)
        .bind(id)
        .fetch_one(db)
        .await
        .context("read conversation")?;
    Ok(row.view(user))
}

#[derive(Debug, Serialize)]
pub struct ConversationPage {
    pub conversations: Vec<ConversationView>,
    /// Pass as `?before=` for the next page; null on the last page.
    pub next_cursor: Option<String>,
}

pub async fn list(
    db: &PgPool,
    user: Uuid,
    before: Option<&str>,
    limit: Option<i64>,
) -> Result<ConversationPage, AppError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let cursor = before.map(Cursor::decode).transpose()?;
    let seller_side = seller_side_sql("$1");
    let rows: Vec<ConversationRow> = sqlx::query_as(&format!(
        "{LIST_SELECT} WHERE (c.buyer_id = $1 OR {seller_side}) \
           AND ($2::timestamptz IS NULL OR (c.last_message_at, c.id) < ($2, $3)) \
         ORDER BY c.last_message_at DESC, c.id DESC LIMIT $4"
    ))
    .bind(user)
    .bind(cursor.as_ref().map(|c| c.0))
    .bind(cursor.as_ref().map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(db)
    .await
    .context("list conversations")?;
    let more = rows.len() as i64 > limit;
    let conversations: Vec<ConversationView> = rows
        .into_iter()
        .take(limit as usize)
        .map(|r| r.view(user))
        .collect();
    let next_cursor = match (more, conversations.last()) {
        (true, Some(last)) => Some(Cursor(last.last_message_at, last.id).encode()?),
        _ => None,
    };
    Ok(ConversationPage {
        conversations,
        next_cursor,
    })
}

/// A delivered message, as both sides see it.
#[derive(Debug, Clone, Serialize)]
pub struct MessageView {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub seq: i64,
    pub from: Party,
    pub body: String,
    /// Which member of the store wrote a store message. Only the store's own side is told.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub sent_at: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct DeliveredRow {
    id: Uuid,
    sender_id: Uuid,
    seq: i64,
    body_enc: Vec<u8>,
    delivered_at: OffsetDateTime,
    sender_name: String,
}

/// Delivered messages after `after`, oldest first. Nothing but `delivered`
/// rows: this is the recipient's view.
pub async fn delivered_after(
    db: &PgPool,
    cipher: &FieldCipher,
    access: &Access,
    after: i64,
    limit: Option<i64>,
) -> Result<Vec<MessageView>, AppError> {
    let rows: Vec<DeliveredRow> = sqlx::query_as(
        "SELECT m.id, m.sender_id, m.seq, m.body_enc, m.delivered_at, \
                u.display_name AS sender_name \
         FROM chat_messages m JOIN users u ON u.id = m.sender_id \
         WHERE m.conversation_id = $1 AND m.status = 'delivered' AND m.seq > $2 \
         ORDER BY m.seq LIMIT $3",
    )
    .bind(access.conversation_id)
    .bind(after)
    .bind(limit.unwrap_or(DEFAULT_MESSAGES).clamp(1, MAX_MESSAGES))
    .fetch_all(db)
    .await
    .context("read messages")?;
    rows.into_iter()
        .map(|r| {
            Ok(MessageView {
                id: r.id,
                conversation_id: access.conversation_id,
                seq: r.seq,
                from: access.party_of(r.sender_id),
                body: cipher.decrypt(&r.body_enc)?,
                // The store's team sees who wrote; the buyer never does.
                by: (access.party == Party::Seller && r.sender_id != access.buyer_id)
                    .then_some(r.sender_name),
                sent_at: r.delivered_at,
            })
        })
        .collect()
}

/// One of the caller's own messages the other side has not seen. A withheld
/// message keeps no body here: its text is only in the encrypted flag.
#[derive(Debug, Serialize)]
pub struct OwnPendingView {
    pub id: Uuid,
    pub client_message_id: Uuid,
    pub status: String,
    pub body: Option<String>,
    pub reason: Option<&'static str>,
    pub flag_id: Option<Uuid>,
    pub suggested: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct OwnPendingRow {
    id: Uuid,
    client_message_id: Uuid,
    status: String,
    body_enc: Option<Vec<u8>>,
    flag_id: Option<Uuid>,
    category: Option<String>,
    suggestion_enc: Option<Vec<u8>>,
    created_at: OffsetDateTime,
}

/// The sender's own held and withheld rows, oldest first. Only ever the
/// caller's: the other side must not learn a message was stopped.
pub async fn own_pending(
    db: &PgPool,
    cipher: &FieldCipher,
    access: &Access,
    user: Uuid,
) -> Result<Vec<OwnPendingView>, AppError> {
    let rows: Vec<OwnPendingRow> = sqlx::query_as(
        "SELECT * FROM ( \
           SELECT m.id, m.client_message_id, m.status, m.body_enc, m.flag_id, f.category, \
                  CASE WHEN f.suggestion_used_at IS NULL \
                        AND f.created_at > now() - make_interval(hours => $3) \
                       THEN f.suggestion_enc END AS suggestion_enc, \
                  m.created_at \
           FROM chat_messages m LEFT JOIN moderation_flags f ON f.id = m.flag_id \
           WHERE m.conversation_id = $1 AND m.sender_id = $2 \
             AND m.status IN ('held', 'withheld') \
           ORDER BY m.created_at DESC LIMIT $4 \
         ) own ORDER BY created_at",
    )
    .bind(access.conversation_id)
    .bind(user)
    .bind(SUGGESTION_HOURS)
    .bind(MAX_MESSAGES)
    .fetch_all(db)
    .await
    .context("read own pending messages")?;
    rows.into_iter()
        .map(|r| {
            Ok(OwnPendingView {
                id: r.id,
                client_message_id: r.client_message_id,
                status: r.status,
                body: r.body_enc.map(|b| cipher.decrypt(&b)).transpose()?,
                reason: r
                    .category
                    .as_deref()
                    .and_then(Category::parse)
                    .map(reason_copy),
                flag_id: r.flag_id,
                suggested: r.suggestion_enc.map(|b| cipher.decrypt(&b)).transpose()?,
                created_at: r.created_at,
            })
        })
        .collect()
}

/// Moves the caller's read mark forward, never back and never past the end.
pub async fn mark_read(db: &PgPool, access: &Access, seq: i64) -> Result<(), AppError> {
    let column = match access.party {
        Party::Buyer => "buyer_read_seq",
        Party::Seller => "seller_read_seq",
    };
    sqlx::query(&format!(
        "UPDATE conversations SET {column} = GREATEST({column}, LEAST($2, last_seq)) \
         WHERE id = $1"
    ))
    .bind(access.conversation_id)
    .bind(seq)
    .execute(db)
    .await
    .context("mark read")?;
    Ok(())
}

/// A delivered message as staff read it in a dispute.
#[derive(Debug, Serialize)]
pub struct StaffMessage {
    pub seq: i64,
    pub from: Party,
    pub body: String,
    #[serde(with = "time::serde::rfc3339")]
    pub sent_at: OffsetDateTime,
}

/// What a blocked or flagged message said, for the Owner only.
#[derive(Debug, Serialize)]
pub struct FlaggedOriginal {
    pub from: Party,
    pub severity: String,
    pub category: String,
    pub original: String,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

/// The order's conversation, delivered messages only, oldest first. Empty when
/// the two never wrote. Staff reach it only through a dispute, and the caller
/// audits that read.
pub async fn staff_transcript(
    db: &PgPool,
    cipher: &FieldCipher,
    order_id: Uuid,
) -> Result<Vec<StaffMessage>, AppError> {
    let rows: Vec<(i64, bool, Vec<u8>, OffsetDateTime)> = sqlx::query_as(
        "SELECT m.seq, m.sender_id = c.buyer_id, m.body_enc, COALESCE(m.delivered_at, m.created_at) \
         FROM chat_messages m JOIN conversations c ON c.id = m.conversation_id \
         WHERE c.order_id = $1 AND m.status = 'delivered' AND m.seq IS NOT NULL \
         ORDER BY m.seq",
    )
    .bind(order_id)
    .fetch_all(db)
    .await
    .context("read staff transcript")?;
    rows.into_iter()
        .map(|(seq, buyer, enc, sent_at)| {
            Ok(StaffMessage {
                seq,
                from: if buyer { Party::Buyer } else { Party::Seller },
                body: cipher.decrypt(&enc)?,
                sent_at,
            })
        })
        .collect()
}

/// The flags raised on the order's conversation, oldest first, with the
/// original text. Owner only: the caller decides who may call this.
pub async fn flagged_originals(
    db: &PgPool,
    cipher: &FieldCipher,
    order_id: Uuid,
) -> Result<Vec<FlaggedOriginal>, AppError> {
    let rows: Vec<(bool, String, String, Vec<u8>, OffsetDateTime)> = sqlx::query_as(
        "SELECT f.user_id = c.buyer_id, f.severity, f.category, f.original_enc, f.created_at \
         FROM moderation_flags f JOIN conversations c ON c.id = f.conversation_id \
         WHERE c.order_id = $1 ORDER BY f.created_at, f.id",
    )
    .bind(order_id)
    .fetch_all(db)
    .await
    .context("read flagged originals")?;
    rows.into_iter()
        .map(|(buyer, severity, category, enc, at)| {
            Ok(FlaggedOriginal {
                from: if buyer { Party::Buyer } else { Party::Seller },
                severity,
                category,
                original: cipher.decrypt(&enc)?,
                at,
            })
        })
        .collect()
}
