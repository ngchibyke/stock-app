//! Flags, strikes and the mute ladder.

use anyhow::{Context, anyhow};
use serde::Serialize;
use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    filter::Category,
    pipeline::{FlagSeverity, Verdict},
};
use crate::{audit, crypto::FieldCipher, error::AppError};

/// At most one strike per user in this many minutes, so one bad burst cannot
/// jump the ladder. Later attempts are still flagged.
const STRIKE_COOLDOWN_MINUTES: i32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Chat,
    Review,
    ReviewReply,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Chat => "chat",
            Surface::Review => "review",
            Surface::ReviewReply => "review_reply",
        }
    }
}

pub struct NewFlag<'a> {
    pub user: Uuid,
    pub surface: Surface,
    pub conversation_id: Option<Uuid>,
    pub order_id: Option<Uuid>,
    pub verdict: &'a Verdict,
    pub original: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StrikeNotice {
    pub active_strikes: i64,
    #[serde(with = "time::serde::rfc3339::option")]
    pub muted_until: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagOutcome {
    pub flag_id: Uuid,
    pub strike: Option<StrikeNotice>,
}

/// Refuses a muted or banned user. A mute stops posting, never reading, and
/// never touches orders or money.
pub async fn ensure_can_post(conn: &mut PgConnection, user: Uuid) -> Result<(), AppError> {
    let row: Option<(bool, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT banned_at IS NOT NULL, muted_until FROM user_moderation \
         WHERE user_id = $1 AND (banned_at IS NOT NULL OR muted_until > now())",
    )
    .bind(user)
    .fetch_optional(&mut *conn)
    .await
    .context("read posting state")?;
    match row {
        None => Ok(()),
        Some((true, _)) => Err(AppError::Forbidden("posting_banned")),
        Some((false, Some(until))) => Err(AppError::Muted(until)),
        Some((false, None)) => Ok(()),
    }
}

/// Stores what was withheld, and on a serious block moves the user up the
/// ladder: warning, then mute, then owner review. Takes the user's row lock,
/// so two blocks at once cannot both pass the one-strike-per-cooldown check.
pub async fn record_flag(
    conn: &mut PgConnection,
    cipher: &FieldCipher,
    flag: NewFlag<'_>,
) -> Result<FlagOutcome, AppError> {
    let (severity, category, source, suggestion) = match flag.verdict {
        Verdict::Clean => return Err(anyhow!("a clean verdict has no flag").into()),
        Verdict::Rewrite {
            category,
            suggestion,
            source,
        } => (
            FlagSeverity::Mild,
            *category,
            *source,
            suggestion.as_deref(),
        ),
        Verdict::Block {
            severity,
            category,
            source,
        } => (*severity, *category, *source, None),
    };
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 FOR NO KEY UPDATE")
        .bind(flag.user)
        .fetch_one(&mut *conn)
        .await
        .context("lock user")?;

    let strike = if severity == FlagSeverity::Serious {
        Some(apply_strike(conn, flag.user, category).await?)
    } else {
        None
    };
    let flag_id: Uuid = sqlx::query_scalar(
        "INSERT INTO moderation_flags (user_id, surface, conversation_id, order_id, severity, \
                category, source, original_enc, suggestion_enc, strike_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
    )
    .bind(flag.user)
    .bind(flag.surface.as_str())
    .bind(flag.conversation_id)
    .bind(flag.order_id)
    .bind(severity.as_str())
    .bind(category.as_str())
    .bind(source.as_str())
    .bind(cipher.encrypt(flag.original))
    .bind(suggestion.map(|s| cipher.encrypt(s)))
    .bind(strike.and_then(|(_, id)| id))
    .fetch_one(&mut *conn)
    .await
    .context("insert moderation flag")?;
    Ok(FlagOutcome {
        flag_id,
        strike: strike.map(|(notice, _)| notice),
    })
}

/// Adds a strike unless one landed inside the cooldown, then applies the
/// ladder. Gives the new strike's id, if any. The caller holds the user lock.
async fn apply_strike(
    conn: &mut PgConnection,
    user: Uuid,
    category: Category,
) -> Result<(StrikeNotice, Option<i64>), AppError> {
    let cooling: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM moderation_strikes \
         WHERE user_id = $1 AND voided_at IS NULL \
           AND at > now() - make_interval(mins => $2))",
    )
    .bind(user)
    .bind(STRIKE_COOLDOWN_MINUTES)
    .fetch_one(&mut *conn)
    .await
    .context("check strike cooldown")?;
    let strike_id = if cooling {
        None
    } else {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO moderation_strikes (user_id, category) VALUES ($1, $2) RETURNING id",
        )
        .bind(user)
        .bind(category.as_str())
        .fetch_one(&mut *conn)
        .await
        .context("insert strike")?;
        audit::record(
            conn,
            None,
            "moderation.strike",
            "user",
            Some(user),
            json!({ "category": category.as_str() }),
        )
        .await?;
        Some(id)
    };

    let active = active_strikes(conn, user).await?;
    let (mute_after, mute_hours, review_after): (i32, i32, i32) = sqlx::query_as(
        "SELECT mute_after_strikes, mute_hours, owner_review_after_strikes \
         FROM platform_settings WHERE id",
    )
    .fetch_one(&mut *conn)
    .await
    .context("read strike settings")?;

    let mut muted_until: Option<OffsetDateTime> = None;
    if strike_id.is_some() && active >= i64::from(mute_after) {
        muted_until = Some(
            sqlx::query_scalar(
                "INSERT INTO user_moderation (user_id, muted_until) \
                 VALUES ($1, now() + make_interval(hours => $2)) \
                 ON CONFLICT (user_id) DO UPDATE SET muted_until = GREATEST( \
                     user_moderation.muted_until, EXCLUDED.muted_until) \
                 RETURNING muted_until",
            )
            .bind(user)
            .bind(mute_hours)
            .fetch_one(&mut *conn)
            .await
            .context("mute user")?,
        );
        audit::record(
            conn,
            None,
            "moderation.muted",
            "user",
            Some(user),
            json!({ "active_strikes": active, "hours": mute_hours }),
        )
        .await?;
    }
    if strike_id.is_some() && active >= i64::from(review_after) {
        sqlx::query(
            "INSERT INTO user_moderation (user_id, owner_review_at) VALUES ($1, now()) \
             ON CONFLICT (user_id) DO UPDATE SET owner_review_at = \
                 COALESCE(user_moderation.owner_review_at, now())",
        )
        .bind(user)
        .execute(&mut *conn)
        .await
        .context("request owner review")?;
    }
    if muted_until.is_none() {
        muted_until = sqlx::query_scalar(
            "SELECT muted_until FROM user_moderation \
             WHERE user_id = $1 AND muted_until > now()",
        )
        .bind(user)
        .fetch_optional(&mut *conn)
        .await
        .context("read mute")?
        .flatten();
    }
    Ok((
        StrikeNotice {
            active_strikes: active,
            muted_until,
        },
        strike_id,
    ))
}

/// Where the user stands now, for telling them about a block again.
pub async fn notice(conn: &mut PgConnection, user: Uuid) -> Result<StrikeNotice, AppError> {
    let active_strikes = active_strikes(conn, user).await?;
    let muted_until = sqlx::query_scalar(
        "SELECT muted_until FROM user_moderation WHERE user_id = $1 AND muted_until > now()",
    )
    .bind(user)
    .fetch_optional(&mut *conn)
    .await
    .context("read mute")?
    .flatten();
    Ok(StrikeNotice {
        active_strikes,
        muted_until,
    })
}

/// Strikes that count: not voided, inside the owner's window.
pub async fn active_strikes(conn: &mut PgConnection, user: Uuid) -> Result<i64, AppError> {
    let count = sqlx::query_scalar(
        "SELECT count(*) FROM moderation_strikes \
         WHERE user_id = $1 AND voided_at IS NULL AND at > now() - make_interval(days => \
             (SELECT strike_window_days FROM platform_settings WHERE id))",
    )
    .bind(user)
    .fetch_one(&mut *conn)
    .await
    .context("count strikes")?;
    Ok(count)
}
