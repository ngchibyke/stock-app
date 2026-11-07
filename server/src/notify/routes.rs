//! The inbox and push-device routes. A push token is personal data: it is
//! encrypted, never logged and never put in an audit row.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use super::kinds::Kind;
use crate::{
    AppState, audit,
    auth::AuthUser,
    crypto::blind_index,
    error::AppError,
    extract::{AppJson, AppQuery},
};

const DEFAULT_PAGE: i64 = 30;
const MAX_PAGE: i64 = 100;
const MAX_READ_IDS: usize = 100;
const MAX_TOKEN: usize = 4096;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/notifications", get(list))
        .route("/me/notifications/read", post(mark_read))
        .route("/me/push-device", put(register).delete(remove))
}

#[derive(Deserialize)]
struct ListQuery {
    /// The id of the last item of the previous page.
    before: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize)]
struct Item {
    id: Uuid,
    kind: Kind,
    title: &'static str,
    body: &'static str,
    order_id: Option<Uuid>,
    dispute_id: Option<Uuid>,
    conversation_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    read: bool,
}

#[derive(Serialize)]
struct Inbox {
    items: Vec<Item>,
    unread: i64,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    kind: String,
    order_id: Option<Uuid>,
    dispute_id: Option<Uuid>,
    conversation_id: Option<Uuid>,
    created_at: OffsetDateTime,
    is_read: bool,
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    AppQuery(q): AppQuery<ListQuery>,
) -> Result<Json<Inbox>, AppError> {
    let limit = q.limit.unwrap_or(DEFAULT_PAGE);
    if !(1..=MAX_PAGE).contains(&limit) {
        return Err(AppError::Validation(format!(
            "limit must be 1 to {MAX_PAGE}"
        )));
    }
    let before: Option<Uuid> = q
        .before
        .as_deref()
        .map(|s| {
            s.parse()
                .map_err(|_| AppError::Validation("before is not a valid id".into()))
        })
        .transpose()?;
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, kind, order_id, dispute_id, conversation_id, created_at, \
                read_at IS NOT NULL AS is_read \
         FROM notifications \
         WHERE user_id = $1 \
           AND ($2::uuid IS NULL OR (created_at, id) < ( \
                (SELECT c.created_at FROM notifications c \
                 WHERE c.id = $2 AND c.user_id = $1), $2)) \
         ORDER BY created_at DESC, id DESC LIMIT $3",
    )
    .bind(user.id)
    .bind(before)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .context("list notifications")?;
    let unread: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id = $1 AND read_at IS NULL",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await
    .context("count unread notifications")?;
    let items = rows
        .into_iter()
        .map(|r| {
            let kind =
                Kind::parse(&r.kind).ok_or_else(|| anyhow::anyhow!("unknown notification kind"))?;
            Ok(Item {
                id: r.id,
                kind,
                title: kind.title(),
                body: kind.body(),
                order_id: r.order_id,
                dispute_id: r.dispute_id,
                conversation_id: r.conversation_id,
                created_at: r.created_at,
                read: r.is_read,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    Ok(Json(Inbox { items, unread }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadBody {
    ids: Option<Vec<Uuid>>,
    all: Option<bool>,
}

async fn mark_read(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<ReadBody>,
) -> Result<StatusCode, AppError> {
    match (body.ids, body.all) {
        (Some(ids), None) => {
            if ids.is_empty() || ids.len() > MAX_READ_IDS {
                return Err(AppError::Validation(format!(
                    "ids must hold 1 to {MAX_READ_IDS} ids"
                )));
            }
            sqlx::query(
                "UPDATE notifications SET read_at = now() \
                 WHERE user_id = $1 AND id = ANY($2) AND read_at IS NULL",
            )
            .bind(user.id)
            .bind(&ids)
            .execute(&state.db)
            .await
            .context("mark notifications read")?;
        }
        (None, Some(true)) => {
            sqlx::query(
                "UPDATE notifications SET read_at = now() \
                 WHERE user_id = $1 AND read_at IS NULL",
            )
            .bind(user.id)
            .execute(&state.db)
            .await
            .context("mark all notifications read")?;
        }
        _ => {
            return Err(AppError::Validation("give either ids or all: true".into()));
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceBody {
    token: String,
    platform: String,
}

impl std::fmt::Debug for DeviceBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceBody(<redacted>)")
    }
}

async fn register(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<DeviceBody>,
) -> Result<StatusCode, AppError> {
    let n = body.token.len();
    if n == 0 || n > MAX_TOKEN || !body.token.chars().all(|c| c.is_ascii_graphic()) {
        return Err(AppError::Validation(format!(
            "token must be 1 to {MAX_TOKEN} printable characters"
        )));
    }
    if body.platform != "android" && body.platform != "ios" {
        return Err(AppError::Validation(
            "platform must be android or ios".into(),
        ));
    }
    let idx = blind_index(&state.index_key, &body.token);
    let enc = state.cipher.encrypt(&body.token);
    let mut tx = state.db.begin().await.context("begin")?;
    // A token seen again under another session or account moves to this one.
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO push_devices (user_id, session_id, platform, token_enc, token_idx) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (token_idx) DO UPDATE SET \
            user_id = EXCLUDED.user_id, session_id = EXCLUDED.session_id, \
            platform = EXCLUDED.platform, token_enc = EXCLUDED.token_enc, \
            registered_at = now(), disabled_at = NULL, disabled_reason = NULL \
         RETURNING id",
    )
    .bind(user.id)
    .bind(user.session_id)
    .bind(&body.platform)
    .bind(enc)
    .bind(idx)
    .fetch_one(&mut *tx)
    .await
    .context("register push device")?;
    audit::record(
        &mut tx,
        Some(user.id),
        "push_device.registered",
        "push_device",
        Some(id),
        serde_json::json!({}),
    )
    .await?;
    tx.commit().await.context("commit")?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(State(state): State<AppState>, user: AuthUser) -> Result<StatusCode, AppError> {
    let mut tx = state.db.begin().await.context("begin")?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE push_devices SET disabled_at = now(), disabled_reason = 'removed' \
         WHERE session_id = $1 AND user_id = $2 AND disabled_at IS NULL RETURNING id",
    )
    .bind(user.session_id)
    .bind(user.id)
    .fetch_all(&mut *tx)
    .await
    .context("disable push devices")?;
    for id in ids {
        audit::record(
            &mut tx,
            Some(user.id),
            "push_device.removed",
            "push_device",
            Some(id),
            serde_json::json!({}),
        )
        .await?;
    }
    tx.commit().await.context("commit")?;
    Ok(StatusCode::NO_CONTENT)
}
