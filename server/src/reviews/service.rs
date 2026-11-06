//! Reviews: one per completed delivery, one seller reply, both moderated.
//!
//! Text is judged outside any transaction, and a review has no held state: if
//! the tone check is down nothing is stored and the buyer tries again. The
//! public shape never carries the buyer's id, name or order.

use anyhow::Context;
use serde::Serialize;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    AppState,
    admin::LOCK_TIMEOUT,
    audit,
    chat::service::SUGGESTION_HOURS,
    checkout::service::busy_err,
    error::AppError,
    limits::{MAX_REPLY_CHARS, MAX_REVIEW_CHARS, MAX_REVIEW_PHOTOS},
    moderation::{
        pipeline::{Verdict, judge, reason_copy},
        strikes::{NewFlag, Surface, ensure_can_post, record_flag},
    },
    orders::views::Cursor,
};

const DEFAULT_PAGE: i64 = 20;
const MAX_PAGE: i64 = 50;
const VIEW_COLUMNS: &str = "id, stars, body, photo_keys, reply, replied_at, created_at";

/// A review as anyone sees it.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ReviewView {
    pub id: Uuid,
    pub stars: i16,
    pub body: Option<String>,
    pub photo_keys: Vec<String>,
    pub reply: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub replied_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "lowercase")]
pub enum PostOutcome {
    Posted {
        review: ReviewView,
    },
    Rewrite {
        flag_id: Uuid,
        suggested: Option<String>,
        reason: &'static str,
    },
    Blocked {
        reason: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        strikes: Option<i64>,
        #[serde(
            with = "time::serde::rfc3339::option",
            skip_serializing_if = "Option::is_none"
        )]
        muted_until: Option<OffsetDateTime>,
    },
}

pub struct NewReview {
    pub stars: i16,
    pub body: Option<String>,
    pub photo_keys: Vec<String>,
    pub use_suggestion: Option<Uuid>,
}

/// What to do with the text, once it is checked.
enum Text {
    None,
    Checked(String),
    Suggestion(Uuid),
}

/// Trims, drops a blank body, and refuses a body beside a suggestion: what is
/// posted then is only ever the server's stored text.
fn clean_text(
    body: Option<&str>,
    suggestion: Option<Uuid>,
    max: usize,
) -> Result<Option<String>, AppError> {
    let text = body.map(str::trim).filter(|t| !t.is_empty());
    if text.is_some() && suggestion.is_some() {
        return Err(AppError::Validation(
            "give either body or use_suggestion".into(),
        ));
    }
    if text.is_some_and(|t| t.chars().count() > max) {
        return Err(AppError::Validation(format!(
            "text must be at most {max} characters"
        )));
    }
    Ok(text.map(str::to_string))
}

/// Judges `text`, storing the flag when it does not pass. `Ok(None)` is clean.
async fn moderate(
    state: &AppState,
    user: Uuid,
    surface: Surface,
    order: Uuid,
    text: &str,
    max: usize,
) -> Result<Option<PostOutcome>, AppError> {
    let verdict = judge(state.tone.as_ref(), text, &[], max)
        .await
        .map_err(|_| AppError::Unavailable("moderation_unavailable"))?;
    if verdict == Verdict::Clean {
        return Ok(None);
    }
    let mut tx = begin(&state.db).await?;
    let flag = record_flag(
        &mut tx,
        &state.cipher,
        NewFlag {
            user,
            surface,
            conversation_id: None,
            order_id: Some(order),
            verdict: &verdict,
            original: text,
        },
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(Some(match verdict {
        Verdict::Rewrite {
            category,
            suggestion,
            ..
        } => PostOutcome::Rewrite {
            flag_id: flag.flag_id,
            suggested: suggestion,
            reason: reason_copy(category),
        },
        Verdict::Block { category, .. } => PostOutcome::Blocked {
            reason: reason_copy(category),
            strikes: flag.strike.map(|s| s.active_strikes),
            muted_until: flag.strike.and_then(|s| s.muted_until),
        },
        Verdict::Clean => unreachable!("returned above"),
    }))
}

pub(crate) async fn begin(db: &PgPool) -> Result<Transaction<'static, Postgres>, AppError> {
    let mut tx = db.begin().await.context("begin")?;
    sqlx::query(LOCK_TIMEOUT)
        .execute(&mut *tx)
        .await
        .context("set lock_timeout")?;
    Ok(tx)
}

/// Spends a suggestion: it must be this user's, for this surface and order,
/// unused and fresh. The flag stays locked until the caller commits, so two
/// posts cannot both spend it.
async fn take_suggestion(
    conn: &mut PgConnection,
    state: &AppState,
    flag: Uuid,
    user: Uuid,
    surface: Surface,
    order: Uuid,
) -> Result<String, AppError> {
    let row: Option<(Option<Vec<u8>>, bool, bool)> = sqlx::query_as(
        "SELECT suggestion_enc, suggestion_used_at IS NOT NULL, \
                created_at <= now() - make_interval(hours => $5) \
         FROM moderation_flags \
         WHERE id = $1 AND user_id = $2 AND surface = $3 AND order_id = $4 \
         FOR UPDATE",
    )
    .bind(flag)
    .bind(user)
    .bind(surface.as_str())
    .bind(order)
    .bind(SUGGESTION_HOURS)
    .fetch_optional(&mut *conn)
    .await
    .map_err(busy_err("review_busy", "lock flag"))?;
    let (Some(encrypted), used, expired) = row.ok_or(AppError::NotFound)? else {
        return Err(AppError::NotFound);
    };
    if used {
        return Err(AppError::Conflict("suggestion_used"));
    }
    if expired {
        return Err(AppError::Conflict("suggestion_expired"));
    }
    sqlx::query("UPDATE moderation_flags SET suggestion_used_at = now() WHERE id = $1")
        .bind(flag)
        .execute(&mut *conn)
        .await
        .context("mark suggestion used")?;
    state.cipher.decrypt(&encrypted)
}

pub async fn post_review(
    state: &AppState,
    user: Uuid,
    order: Uuid,
    new: NewReview,
) -> Result<PostOutcome, AppError> {
    if !(1..=5).contains(&new.stars) {
        return Err(AppError::Validation("stars must be 1 to 5".into()));
    }
    if new.photo_keys.len() > MAX_REVIEW_PHOTOS {
        return Err(AppError::Validation(format!(
            "photo_keys must contain at most {MAX_REVIEW_PHOTOS} entries"
        )));
    }
    if new.photo_keys.iter().any(|k| k.trim().is_empty()) {
        return Err(AppError::Validation(
            "photo_keys must not contain empty entries".into(),
        ));
    }
    let body = clean_text(new.body.as_deref(), new.use_suggestion, MAX_REVIEW_CHARS)?;

    let row: Option<(Uuid, bool, bool)> = sqlx::query_as(
        "SELECT o.store_id, o.status = 'completed' AND o.delivered_at IS NOT NULL, \
                EXISTS (SELECT 1 FROM reviews r WHERE r.order_id = o.id) \
         FROM orders o WHERE o.id = $1 AND o.buyer_id = $2",
    )
    .bind(order)
    .bind(user)
    .fetch_optional(&state.db)
    .await
    .context("read order for review")?;
    let (store, eligible, reviewed) = row.ok_or(AppError::NotFound)?;
    if !eligible {
        return Err(AppError::Conflict("review_not_allowed"));
    }
    if reviewed {
        return Err(AppError::Conflict("already_reviewed"));
    }
    let mut conn = state.db.acquire().await.context("acquire")?;
    ensure_can_post(&mut conn, user).await?;
    drop(conn);

    let text = match (body, new.use_suggestion) {
        (Some(body), _) => {
            let outcome =
                moderate(state, user, Surface::Review, order, &body, MAX_REVIEW_CHARS).await?;
            if let Some(outcome) = outcome {
                return Ok(outcome);
            }
            Text::Checked(body)
        }
        (None, Some(flag)) => Text::Suggestion(flag),
        (None, None) => Text::None,
    };

    let mut tx = begin(&state.db).await?;
    ensure_can_post(&mut tx, user).await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM stores WHERE id = $1 FOR NO KEY UPDATE")
        .bind(store)
        .fetch_one(&mut *tx)
        .await
        .map_err(busy_err("review_busy", "lock store"))?;
    let body = match text {
        Text::None => None,
        Text::Checked(body) => Some(body),
        Text::Suggestion(flag) => {
            Some(take_suggestion(&mut tx, state, flag, user, Surface::Review, order).await?)
        }
    };
    let review: Option<ReviewView> = sqlx::query_as(&format!(
        "INSERT INTO reviews (order_id, store_id, buyer_id, stars, body, photo_keys) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (order_id) DO NOTHING RETURNING {VIEW_COLUMNS}"
    ))
    .bind(order)
    .bind(store)
    .bind(user)
    .bind(new.stars)
    .bind(body)
    .bind(&new.photo_keys)
    .fetch_optional(&mut *tx)
    .await
    .context("insert review")?;
    // Rolling back here also gives an unused suggestion back.
    let review = review.ok_or(AppError::Conflict("already_reviewed"))?;
    recount(&mut tx, store).await?;
    audit::record(
        &mut tx,
        Some(user),
        "review.posted",
        "order",
        Some(order),
        serde_json::json!({ "stars": new.stars, "store_id": store }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(PostOutcome::Posted { review })
}

pub async fn post_reply(
    state: &AppState,
    user: Uuid,
    review: Uuid,
    body: Option<&str>,
    use_suggestion: Option<Uuid>,
) -> Result<PostOutcome, AppError> {
    let body = clean_text(body, use_suggestion, MAX_REPLY_CHARS)?;
    if body.is_none() && use_suggestion.is_none() {
        return Err(AppError::Validation("give body or use_suggestion".into()));
    }
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT r.order_id, r.reply IS NOT NULL \
         FROM reviews r JOIN stores s ON s.id = r.store_id \
         WHERE r.id = $1 AND s.owner_id = $2 AND r.hidden_at IS NULL",
    )
    .bind(review)
    .bind(user)
    .fetch_optional(&state.db)
    .await
    .context("read review for reply")?;
    let (order, replied) = row.ok_or(AppError::NotFound)?;
    if replied {
        return Err(AppError::Conflict("already_replied"));
    }
    let mut conn = state.db.acquire().await.context("acquire")?;
    ensure_can_post(&mut conn, user).await?;
    drop(conn);

    if let Some(body) = &body {
        let outcome = moderate(
            state,
            user,
            Surface::ReviewReply,
            order,
            body,
            MAX_REPLY_CHARS,
        )
        .await?;
        if let Some(outcome) = outcome {
            return Ok(outcome);
        }
    }

    let mut tx = begin(&state.db).await?;
    ensure_can_post(&mut tx, user).await?;
    let reply = match (body, use_suggestion) {
        (Some(body), _) => body,
        (None, Some(flag)) => {
            take_suggestion(&mut tx, state, flag, user, Surface::ReviewReply, order).await?
        }
        (None, None) => unreachable!("refused above"),
    };
    let view: Option<ReviewView> = sqlx::query_as(&format!(
        "UPDATE reviews SET reply = $2, replied_at = now() \
         WHERE id = $1 AND reply IS NULL RETURNING {VIEW_COLUMNS}"
    ))
    .bind(review)
    .bind(reply)
    .fetch_optional(&mut *tx)
    .await
    .map_err(busy_err("review_busy", "store reply"))?;
    let view = view.ok_or(AppError::Conflict("already_replied"))?;
    audit::record(
        &mut tx,
        Some(user),
        "review.replied",
        "order",
        Some(order),
        serde_json::json!({ "review_id": review }),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(PostOutcome::Posted { review: view })
}

/// Sets both rating columns from the visible reviews. The only writer, so the
/// numbers cannot drift. The caller holds the store's row lock.
pub async fn recount(conn: &mut PgConnection, store_id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE stores s SET rating_count = r.n, rating_sum = r.total \
         FROM (SELECT count(*)::int AS n, COALESCE(sum(stars), 0)::bigint AS total \
               FROM reviews WHERE store_id = $1 AND hidden_at IS NULL) r \
         WHERE s.id = $1",
    )
    .bind(store_id)
    .execute(&mut *conn)
    .await
    .context("recount ratings")?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct ReviewPage {
    pub reviews: Vec<ReviewView>,
    /// Pass as `?before=` for the next page; null on the last page.
    pub next_cursor: Option<String>,
}

pub async fn list_for_store(
    db: &PgPool,
    store: Uuid,
    before: Option<&str>,
    limit: Option<i64>,
) -> Result<ReviewPage, AppError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let cursor = before.map(Cursor::decode).transpose()?;
    let mut reviews: Vec<ReviewView> = sqlx::query_as(&format!(
        "SELECT {VIEW_COLUMNS} FROM reviews \
         WHERE store_id = $1 AND hidden_at IS NULL \
           AND ($2::timestamptz IS NULL OR (created_at, id) < ($2, $3)) \
         ORDER BY created_at DESC, id DESC LIMIT $4"
    ))
    .bind(store)
    .bind(cursor.as_ref().map(|c| c.0))
    .bind(cursor.as_ref().map(|c| c.1))
    .bind(limit + 1)
    .fetch_all(db)
    .await
    .context("list reviews")?;
    let more = reviews.len() as i64 > limit;
    reviews.truncate(limit as usize);
    let next_cursor = match (more, reviews.last()) {
        (true, Some(last)) => Some(Cursor(last.created_at, last.id).encode()?),
        _ => None,
    };
    Ok(ReviewPage {
        reviews,
        next_cursor,
    })
}
