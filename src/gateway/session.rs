//! Session store — holds every connected WebSocket session.
//!
//! This solves Problem #2: lock contention.
//!
//! A normal HashMap with RwLock makes 5000 users fight for one lock.
//! DashMap splits the data into 64 shards (like 64 mini-HashMaps).
//! Alice on shard #12 and Bob on shard #47 never block each other.
//!
//! Each session also gets its own mpsc channel — a one-way pipe.
//! Sending a message to Bob = dropping it into his pipe. No locks,
//! no shared buffers. His WebSocket task picks it up on the other end.

use crate::models::*;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// A single WebSocket session for one connected device.
///
/// A user can have multiple sessions (phone + desktop).
/// Each session has its own `sender` channel — this is how we
/// deliver messages without locking shared state.
#[derive(Debug)]
pub struct Session {
    pub session_id: SessionId,
    pub user_id: UserId,
    pub username: String,
    pub connected_at: DateTime<Utc>,

    /// Atomic timestamp of the last heartbeat. The heartbeat monitor
    /// (Problem #7) reads this to detect dead connections. AtomicU64
    /// means no lock is needed — both the WebSocket task and the
    /// monitor can access it simultaneously.
    pub last_heartbeat: Arc<AtomicU64>,

    /// One-way pipe to this session's WebSocket.
    /// Calling sender.send(event) puts the event into the pipe.
    /// The WebSocket task on the other end picks it up and sends
    /// it to the user's browser. This takes ~0.001ms — no network,
    /// no lock, just a memory write.
    pub sender: mpsc::UnboundedSender<ServerEvent>,
}

impl Session {
    pub fn new(
        user_id: UserId,
        username: String,
        sender: mpsc::UnboundedSender<ServerEvent>,
    ) -> Self {
        Self {
            session_id: uuid::Uuid::new_v4(),
            user_id,
            username,
            connected_at: Utc::now(),
            last_heartbeat: Arc::new(AtomicU64::new(Utc::now().timestamp() as u64)),
            sender,
        }
    }

    /// Update the heartbeat timestamp. Called on every client message.
    pub fn touch(&self) {
        self.last_heartbeat
            .store(Utc::now().timestamp() as u64, Ordering::Relaxed);
    }

    /// How many seconds since the last heartbeat.
    pub fn seconds_since_heartbeat(&self) -> u64 {
        let now = Utc::now().timestamp() as u64;
        let last = self.last_heartbeat.load(Ordering::Relaxed);
        now.saturating_sub(last)
    }

    /// Try to send an event to this session. Returns false if the
    /// channel is closed (meaning the WebSocket task has ended).
    pub fn send(&self, event: ServerEvent) -> bool {
        self.sender.send(event).is_ok()
    }
}

/// Thread-safe registry of all sessions on this gateway node.
///
/// The key data structure: `DashMap<UserId, Vec<Session>>`
///
/// Why DashMap instead of RwLock<HashMap>?
///   - RwLock: 1 lock for the whole map. 5000 users = 1 door.
///   - DashMap: 64 shards. 5000 users = 64 doors. Almost no contention.
///
/// Why Vec<Session> per user?
///   - A user can be on phone AND desktop simultaneously.
///   - Each device = one session in the Vec.
///   - When we deliver a message to a user, we push to ALL their sessions.
pub struct SessionStore {
    sessions: DashMap<UserId, Vec<Session>>,
    total_connections: AtomicU64,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            total_connections: AtomicU64::new(0),
        }
    }

    /// Register a new session. Called when a user authenticates.
    pub fn insert(&self, session: Session) -> SessionId {
        let sid = session.session_id;
        let uid = session.user_id;
        self.sessions.entry(uid).or_default().push(session);
        self.total_connections.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(user_id = %uid, session_id = %sid, "Session registered");
        sid
    }

    /// Remove a specific session. Called on disconnect or heartbeat timeout.
    ///
    /// In Rust, when we remove the Session from the Vec, the struct is
    /// dropped immediately — memory freed right here, not during a GC
    /// cycle 2 minutes later. That's Problem #1 solved.
    pub fn remove(&self, user_id: UserId, session_id: SessionId) -> bool {
        let mut removed = false;
        if let Some(mut sessions) = self.sessions.get_mut(&user_id) {
            let before = sessions.len();
            sessions.retain(|s| s.session_id != session_id);
            removed = sessions.len() < before;

            // If this was the user's last session, remove the map entry entirely.
            if sessions.is_empty() {
                drop(sessions); // Release the DashMap lock first
                self.sessions.remove(&user_id);
            }
        }
        if removed {
            self.total_connections.fetch_sub(1, Ordering::Relaxed);
            tracing::debug!(user_id = %user_id, session_id = %session_id, "Session removed");
        }
        removed
    }

    /// Deliver an event to every session a user has on this node.
    /// If they're on phone + desktop, both get it.
    pub fn send_to_user(&self, user_id: &UserId, event: &ServerEvent) -> usize {
        let mut sent = 0;
        if let Some(sessions) = self.sessions.get(user_id) {
            for session in sessions.iter() {
                if session.send(event.clone()) {
                    sent += 1;
                }
            }
        }
        sent
    }

    /// Deliver an event to a list of users. Used for message fan-out.
    pub fn send_to_users(&self, user_ids: &[UserId], event: &ServerEvent) -> usize {
        user_ids.iter().map(|uid| self.send_to_user(uid, event)).sum()
    }

    /// Check if a user has any active sessions on this node.
    pub fn is_connected(&self, user_id: &UserId) -> bool {
        self.sessions
            .get(user_id)
            .map_or(false, |s| !s.is_empty())
    }

    /// Total active connections across all users.
    pub fn total_connections(&self) -> u64 {
        self.total_connections.load(Ordering::Relaxed)
    }

    /// All user IDs with at least one active session.
    pub fn connected_user_ids(&self) -> Vec<UserId> {
        self.sessions.iter().map(|entry| *entry.key()).collect()
    }

    /// Find sessions that haven't sent a heartbeat in `timeout_secs`.
    /// Called by the heartbeat monitor (Problem #7).
    pub fn find_stale_sessions(&self, timeout_secs: u64) -> Vec<(UserId, SessionId)> {
        let mut stale = Vec::new();
        for entry in self.sessions.iter() {
            for session in entry.value().iter() {
                if session.seconds_since_heartbeat() > timeout_secs {
                    stale.push((*entry.key(), session.session_id));
                }
            }
        }
        stale
    }

    /// Number of sessions for a specific user.
    pub fn session_count(&self, user_id: &UserId) -> usize {
        self.sessions.get(user_id).map_or(0, |s| s.len())
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}
