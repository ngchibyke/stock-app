//! The live socket. It only pushes: sending goes through REST, so outcomes,
//! idempotency and rate limits live in one place. Anything a client sends here
//! other than a pong or a close is ignored.
//!
//! Auth is the normal bearer header, never a token in the URL, where it would
//! end up in logs. The session is checked again on a timer, so a signed-out
//! user stops listening within [`SocketTimings::session_check`].

use std::time::Duration;

use anyhow::Context;
use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket, rejection::WebSocketUpgradeRejection},
    },
    http::HeaderMap,
    response::Response,
};
use tokio::time::{MissedTickBehavior, interval, timeout};

use super::hub::{Frame, SocketTimings, Subscription};
use crate::{
    AppState,
    auth::{AuthUser, extractor::bearer_token, sessions::hash_token},
    error::AppError,
};

/// Client frames are only pongs and closes, so anything larger is misuse.
const MAX_INCOMING_BYTES: usize = 4096;
/// A client that takes longer than this to accept a frame is dropped.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn connect(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, AppError> {
    let upgrade =
        upgrade.map_err(|_| AppError::Validation("this route needs a WebSocket upgrade".into()))?;
    let token_hash = hash_token(bearer_token(&headers).ok_or(AppError::Unauthorized)?);
    let subscription = state
        .chat_hub
        .subscribe(user.id)
        .map_err(|_| AppError::TooManyRequests("too_many_sockets"))?;
    Ok(upgrade
        .max_message_size(MAX_INCOMING_BYTES)
        .on_upgrade(move |socket| serve(socket, subscription, state, token_hash)))
}

async fn serve(
    mut socket: WebSocket,
    mut subscription: Subscription,
    state: AppState,
    token_hash: Vec<u8>,
) {
    let SocketTimings {
        ping,
        session_check,
    } = state.chat_hub.timings();
    let mut ping = interval(ping);
    let mut check = interval(session_check);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);
    check.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // Intervals fire at once; the upgrade has only just proved both of these.
    ping.tick().await;
    check.tick().await;
    let stopped = state.chat_hub.stopped().clone();
    let mut awaiting_pong = false;

    loop {
        tokio::select! {
            _ = stopped.cancelled() => break,
            frame = subscription.recv() => {
                let Some(frame) = frame else { break };
                if !push(&mut socket, &frame).await {
                    return;
                }
            }
            _ = ping.tick() => {
                if awaiting_pong {
                    break;
                }
                awaiting_pong = true;
                if !send(&mut socket, Message::Ping(Vec::new().into())).await {
                    return;
                }
            }
            _ = check.tick() => {
                // Unknown counts as gone: the client simply reconnects.
                if !matches!(session_alive(&state, &token_hash).await, Ok(true)) {
                    break;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Pong(_))) => awaiting_pong = false,
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
    send(&mut socket, Message::Close(None)).await;
}

async fn push(socket: &mut WebSocket, frame: &Frame) -> bool {
    match serde_json::to_string(frame) {
        Ok(text) => send(socket, Message::text(text)).await,
        Err(_) => false,
    }
}

async fn send(socket: &mut WebSocket, message: Message) -> bool {
    matches!(
        timeout(SEND_TIMEOUT, socket.send(message)).await,
        Ok(Ok(()))
    )
}

async fn session_alive(state: &AppState, token_hash: &[u8]) -> Result<bool, AppError> {
    let alive = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now() \
           AND u.deleted_at IS NULL)",
    )
    .bind(token_hash)
    .fetch_one(&state.db)
    .await
    .context("check session")?;
    Ok(alive)
}
