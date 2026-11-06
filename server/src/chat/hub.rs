//! Pushes frames to the sockets a user has open on this server.
//!
//! One process only, by design for launch. A frame is a nudge, never the
//! record: anything dropped here is still in the database, and a client catches
//! up over REST when it reconnects.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::Serialize;
use time::OffsetDateTime;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::service::MessageView;

pub const MAX_SOCKETS_PER_USER: usize = 5;
pub const QUEUE_PER_SOCKET: usize = 64;

/// The result of sending a message, shared by the send route, the socket and
/// the held-message job.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SendOutcomeView {
    /// `delivered`, `held`, `blocked` or `rewrite`.
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<MessageView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strikes: Option<i64>,
    #[serde(
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub muted_until: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Frame {
    Message(MessageView),
    Moderation {
        conversation_id: Uuid,
        message_id: Uuid,
        outcome: SendOutcomeView,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("too many sockets")]
pub struct TooManySockets;

struct Socket {
    id: u64,
    tx: mpsc::Sender<Frame>,
}

#[derive(Default)]
struct Registry {
    sockets: Mutex<HashMap<Uuid, Vec<Socket>>>,
    next_id: AtomicU64,
}

impl Registry {
    /// The lock is never held across an await, and a panic elsewhere leaves
    /// the map usable.
    fn lock(&self) -> MutexGuard<'_, HashMap<Uuid, Vec<Socket>>> {
        self.sockets.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct ChatHub {
    registry: Arc<Registry>,
    stopped: CancellationToken,
}

impl Default for ChatHub {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatHub {
    pub fn new() -> Self {
        Self {
            registry: Default::default(),
            stopped: CancellationToken::new(),
        }
    }

    pub fn subscribe(&self, user: Uuid) -> Result<Subscription, TooManySockets> {
        let (tx, rx) = mpsc::channel(QUEUE_PER_SOCKET);
        let id = self.registry.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut sockets = self.registry.lock();
            let open = sockets.entry(user).or_default();
            if open.len() >= MAX_SOCKETS_PER_USER {
                return Err(TooManySockets);
            }
            open.push(Socket { id, tx });
        }
        Ok(Subscription {
            id,
            user,
            rx,
            registry: self.registry.clone(),
            stopped: self.stopped.clone(),
        })
    }

    /// Never waits. A socket whose queue is full is dropped from the hub: it
    /// drains what it has, ends, and the client catches up over REST.
    pub fn publish(&self, user: Uuid, frame: Frame) {
        let mut sockets = self.registry.lock();
        let Some(open) = sockets.get_mut(&user) else {
            return;
        };
        open.retain(|socket| socket.tx.try_send(frame.clone()).is_ok());
        if open.is_empty() {
            sockets.remove(&user);
        }
    }

    /// Ends every subscription. Called when the shutdown signal fires.
    pub fn stop(&self) {
        self.stopped.cancel();
        self.registry.lock().clear();
    }

    pub fn stopped(&self) -> &CancellationToken {
        &self.stopped
    }

    #[cfg(test)]
    fn socket_count(&self, user: Uuid) -> usize {
        self.registry.lock().get(&user).map_or(0, Vec::len)
    }

    #[cfg(test)]
    fn user_count(&self) -> usize {
        self.registry.lock().len()
    }
}

/// One open socket. Removes itself from the hub on drop.
pub struct Subscription {
    id: u64,
    user: Uuid,
    rx: mpsc::Receiver<Frame>,
    registry: Arc<Registry>,
    stopped: CancellationToken,
}

impl Subscription {
    /// The next frame, or `None` once the socket should close.
    pub async fn recv(&mut self) -> Option<Frame> {
        tokio::select! {
            _ = self.stopped.cancelled() => None,
            frame = self.rx.recv() => frame,
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut sockets = self.registry.lock();
        if let Some(open) = sockets.get_mut(&self.user) {
            open.retain(|socket| socket.id != self.id);
            if open.is_empty() {
                sockets.remove(&self.user);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;
    use crate::chat::service::Party;

    fn frame(seq: i64) -> Frame {
        Frame::Message(MessageView {
            id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            seq,
            from: Party::Buyer,
            body: "hello".into(),
            sent_at: OffsetDateTime::UNIX_EPOCH,
        })
    }

    async fn next(sub: &mut Subscription) -> Option<Frame> {
        timeout(Duration::from_millis(200), sub.recv())
            .await
            .expect("recv should resolve")
    }

    fn seq(frame: Option<Frame>) -> i64 {
        match frame {
            Some(Frame::Message(m)) => m.seq,
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn publish_reaches_every_socket_of_that_user_only() {
        let hub = ChatHub::new();
        let (ada, bola) = (Uuid::new_v4(), Uuid::new_v4());
        let mut a1 = hub.subscribe(ada).unwrap();
        let mut a2 = hub.subscribe(ada).unwrap();
        let mut b = hub.subscribe(bola).unwrap();
        hub.publish(ada, frame(7));
        assert_eq!(seq(next(&mut a1).await), 7);
        assert_eq!(seq(next(&mut a2).await), 7);
        assert!(
            timeout(Duration::from_millis(50), b.recv()).await.is_err(),
            "another user got the frame"
        );
    }

    #[tokio::test]
    async fn sixth_socket_is_refused() {
        let hub = ChatHub::new();
        let user = Uuid::new_v4();
        let _open: Vec<_> = (0..MAX_SOCKETS_PER_USER)
            .map(|_| hub.subscribe(user).unwrap())
            .collect();
        assert!(matches!(hub.subscribe(user), Err(TooManySockets)));
        assert!(hub.subscribe(Uuid::new_v4()).is_ok());
    }

    #[tokio::test]
    async fn slow_socket_is_dropped_not_awaited() {
        let hub = ChatHub::new();
        let user = Uuid::new_v4();
        let mut slow = hub.subscribe(user).unwrap();
        let fast_user = Uuid::new_v4();
        let _fast = hub.subscribe(fast_user).unwrap();
        // One past the queue: publish must return at once every time.
        for i in 0..=QUEUE_PER_SOCKET as i64 {
            hub.publish(user, frame(i));
        }
        assert_eq!(hub.socket_count(user), 0);
        // What was queued drains, then the socket ends, so the client reconnects.
        for _ in 0..QUEUE_PER_SOCKET {
            assert!(next(&mut slow).await.is_some());
        }
        assert!(next(&mut slow).await.is_none());
        assert_eq!(hub.socket_count(fast_user), 1);
    }

    #[tokio::test]
    async fn dropping_a_subscription_cleans_up() {
        let hub = ChatHub::new();
        let user = Uuid::new_v4();
        let first = hub.subscribe(user).unwrap();
        let second = hub.subscribe(user).unwrap();
        drop(first);
        assert_eq!(hub.socket_count(user), 1);
        drop(second);
        assert_eq!(hub.user_count(), 0, "an empty entry was left behind");
    }

    #[tokio::test]
    async fn stop_ends_every_subscription() {
        let hub = ChatHub::new();
        let mut a = hub.subscribe(Uuid::new_v4()).unwrap();
        let mut b = hub.subscribe(Uuid::new_v4()).unwrap();
        assert!(!hub.stopped().is_cancelled());
        hub.stop();
        assert!(hub.stopped().is_cancelled());
        assert!(next(&mut a).await.is_none());
        assert!(next(&mut b).await.is_none());
    }
}
