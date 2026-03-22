//! Presence Tracker — tracks who's online, idle, or offline.
//!
//! This solves Problem #4: presence flickering.
//!
//! When Charlie's phone goes through a tunnel and loses wifi for
//! 2 seconds, we don't want to broadcast "Charlie is offline" to
//! 50,000 users and then immediately "Charlie is online" when he
//! reconnects. That's 100,000 wasted notifications.
//!
//! The fix: when all of a user's sessions close, we start a 5-second
//! timer. If they reconnect within 5 seconds, we cancel the timer
//! silently — nobody knows anything happened. If 5 seconds pass with
//! no reconnection, THEN we broadcast offline.
//!
//! We use tokio::select! to race the timer against the cancellation
//! signal. Whichever fires first wins.

use crate::models::*;
use crate::presence::broadcast::PresenceBroadcaster;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::sleep;

/// Per-user state tracked in memory.
#[derive(Clone)]
struct UserPresence {
    status: PresenceStatus,
    session_count: u32,
}

pub struct PresenceTracker {
    /// Current presence state for each user on this node.
    state: DashMap<UserId, UserPresence>,

    /// Handles the expensive fan-out (Problem #5).
    broadcaster: Arc<PresenceBroadcaster>,

    /// How long to wait before declaring someone offline.
    offline_debounce: Duration,

    /// Cancel tokens for pending offline timers.
    /// When a user reconnects, we look up their token here and
    /// fire it to cancel the pending offline broadcast.
    pending_offline: DashMap<UserId, Arc<Notify>>,
}

impl PresenceTracker {
    pub fn new(broadcaster: Arc<PresenceBroadcaster>, offline_debounce: Duration) -> Self {
        Self {
            state: DashMap::new(),
            broadcaster,
            offline_debounce,
            pending_offline: DashMap::new(),
        }
    }

    /// Called when a new WebSocket session opens.
    ///
    /// If there's a pending offline timer (the user was briefly
    /// disconnected), cancel it immediately. Then set status to online.
    pub async fn on_session_open(&self, user_id: UserId) {
        // Cancel any pending offline timer — user is back!
        if let Some((_, notify)) = self.pending_offline.remove(&user_id) {
            notify.notify_one(); // This triggers the cancel path in select!
        }

        let mut changed = false;
        self.state
            .entry(user_id)
            .and_modify(|p| {
                p.session_count += 1;
                if p.status == PresenceStatus::Offline {
                    p.status = PresenceStatus::Online;
                    changed = true;
                }
            })
            .or_insert_with(|| {
                changed = true;
                UserPresence {
                    status: PresenceStatus::Online,
                    session_count: 1,
                }
            });

        if changed {
            self.broadcaster
                .broadcast(user_id, PresenceStatus::Online)
                .await;
        }
    }

    /// Called when ALL of a user's sessions have closed.
    ///
    /// This is where the debounce magic happens. We DON'T go offline
    /// immediately. Instead we start a timer and wait.
    pub async fn on_all_sessions_closed(&self, user_id: UserId) {
        // Cancel any existing timer first (in case of rapid disconnect/reconnect).
        if let Some((_, notify)) = self.pending_offline.remove(&user_id) {
            notify.notify_one();
        }

        // Create a new cancellation token.
        let cancel = Arc::new(Notify::new());
        self.pending_offline.insert(user_id, cancel.clone());

        let debounce = self.offline_debounce;
        let state = self.state.clone();
        let broadcaster = self.broadcaster.clone();

        // Spawn a task that races the timer against the cancel signal.
        tokio::spawn(async move {
            tokio::select! {
                // Path A: Timer expires — user is genuinely offline.
                _ = sleep(debounce) => {
                    if let Some(mut entry) = state.get_mut(&user_id) {
                        entry.status = PresenceStatus::Offline;
                        entry.session_count = 0;
                    }
                    broadcaster.broadcast(user_id, PresenceStatus::Offline).await;
                    tracing::debug!(user_id = %user_id, "User went offline (debounced)");
                }
                // Path B: User reconnected — cancel silently.
                _ = cancel.notified() => {
                    // Do absolutely nothing. Nobody was notified.
                    // Zero traffic wasted.
                    tracing::debug!(user_id = %user_id, "Offline timer cancelled (reconnected)");
                }
            }
        });
    }

    /// Explicitly set a user's status (e.g., when they choose DND or idle).
    pub async fn set_status(&self, user_id: UserId, status: PresenceStatus) {
        let mut changed = false;
        self.state.entry(user_id).and_modify(|p| {
            if p.status != status {
                p.status = status;
                changed = true;
            }
        });

        if changed {
            self.broadcaster.broadcast(user_id, status).await;
        }
    }

    /// Get a user's current presence.
    pub fn get_status(&self, user_id: &UserId) -> PresenceStatus {
        self.state
            .get(user_id)
            .map(|p| p.status)
            .unwrap_or(PresenceStatus::Offline)
    }
}
