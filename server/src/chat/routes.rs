//! Chat routes. Sending is a plain request that answers with the outcome; the
//! WebSocket only pushes.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    hub::SendOutcomeView,
    send,
    service::{self, ConversationPage, ConversationView, MessageView, OwnPendingView, Subject},
    ws,
};
use crate::{
    AppState,
    admin::parse_id,
    auth::AuthUser,
    error::AppError,
    extract::{AppJson, AppQuery},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/chats", post(open).get(list))
        .route("/chats/ws", get(ws::connect))
        .route("/chats/{id}/messages", get(messages).post(send_message))
        .route(
            "/chats/{id}/suggestions/{flag_id}/send",
            post(send_suggestion),
        )
        .route("/chats/{id}/read", post(read))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenBody {
    product_id: Option<Uuid>,
    order_id: Option<Uuid>,
}

async fn open(
    State(state): State<AppState>,
    user: AuthUser,
    AppJson(body): AppJson<OpenBody>,
) -> Result<Json<ConversationView>, AppError> {
    let subject = match (body.product_id, body.order_id) {
        (Some(product), None) => Subject::Product(product),
        (None, Some(order)) => Subject::Order(order),
        _ => {
            return Err(AppError::Validation(
                "give either product_id or order_id".into(),
            ));
        }
    };
    Ok(Json(service::open(&state.db, user.id, subject).await?))
}

#[derive(Deserialize)]
struct ListQuery {
    before: Option<String>,
    limit: Option<i64>,
}

async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    AppQuery(q): AppQuery<ListQuery>,
) -> Result<Json<ConversationPage>, AppError> {
    Ok(Json(
        service::list(&state.db, user.id, q.before.as_deref(), q.limit).await?,
    ))
}

#[derive(Deserialize)]
struct MessagesQuery {
    after: Option<i64>,
    limit: Option<i64>,
}

#[derive(Serialize)]
struct MessagesView {
    messages: Vec<MessageView>,
    /// The caller's own held and withheld messages.
    pending: Vec<OwnPendingView>,
}

async fn messages(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    AppQuery(q): AppQuery<MessagesQuery>,
) -> Result<Json<MessagesView>, AppError> {
    let access = service::access(&state.db, user.id, parse_id(&id)?).await?;
    let messages = service::delivered_after(
        &state.db,
        &state.cipher,
        &access,
        q.after.unwrap_or(0),
        q.limit,
    )
    .await?;
    let pending = service::own_pending(&state.db, &state.cipher, &access, user.id).await?;
    Ok(Json(MessagesView { messages, pending }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadBody {
    seq: i64,
}

async fn read(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    AppJson(body): AppJson<ReadBody>,
) -> Result<StatusCode, AppError> {
    let access = service::access(&state.db, user.id, parse_id(&id)?).await?;
    service::mark_read(&state.db, &access, body.seq).await?;
    let mut conn = state.db.acquire().await.map_err(anyhow::Error::from)?;
    crate::notify::service::mark_conversation_read(&mut conn, user.id, access.conversation_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendBody {
    client_message_id: Uuid,
    body: String,
}

async fn send_message(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    AppJson(body): AppJson<SendBody>,
) -> Result<Json<SendOutcomeView>, AppError> {
    let outcome = send::send(
        &state,
        user.id,
        parse_id(&id)?,
        body.client_message_id,
        &body.body,
    )
    .await?;
    Ok(Json(outcome))
}

/// Takes no text: what is sent is what the server stored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SuggestionBody {
    client_message_id: Uuid,
}

async fn send_suggestion(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, flag)): Path<(String, String)>,
    AppJson(body): AppJson<SuggestionBody>,
) -> Result<Json<SendOutcomeView>, AppError> {
    let outcome = send::send_suggested(
        &state,
        user.id,
        parse_id(&id)?,
        parse_id(&flag)?,
        body.client_message_id,
    )
    .await?;
    Ok(Json(outcome))
}
