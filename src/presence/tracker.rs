//! Presence Tracker — tracks who's online, idle, or offline.
//!
//! Solves Problem #4: presence flickering.

use crate::models::*;
use crate::presence::broadcast::PresenceBroadcaster;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::sleep;

struct UserPresence {
    status: PresenceStatus,
    session_count: u32,
}

pub struct PresenceTracker {
    // Arc wraps the DashMap so the spawned debounce task and the
    // main tracker share the SAME map, not a copy.
    state: Arc<DashMap<UserId, UserPresence>>,
    broadcaster: Arc<PresenceBroadcaster>,
    offline_debounce: Duration,
    pending_offline: Arc<DashMap<UserId, Arc<Notify>>>,
}

impl PresenceTracker {
    pub fn new(broadcaster: Arc<PresenceBroadcaster>, offline_debounce: Duration) -> Self {
        Self {
            state: Arc::new(DashMap::new()),
            broadcaster,
            offline_debounce,
            pending_offline: Arc::new(DashMap::new()),
        }
    }

    pub async fn on_session_open(&self, user_id: UserId) {
        if let Some((_, notify)) = self.pending_offline.remove(&user_id) {
            notify.notify_one();
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

    pub async fn on_all_sessions_closed(&self, user_id: UserId) {
        if let Some((_, notify)) = self.pending_offline.remove(&user_id) {
            notify.notify_one();
        }

        let cancel = Arc::new(Notify::new());
        self.pending_offline.insert(user_id, cancel.clone());

        let debounce = self.offline_debounce;
        let state = Arc::clone(&self.state);
        let broadcaster = self.broadcaster.clone();
        let pending = Arc::clone(&self.pending_offline);

        tokio::spawn(async move {
            tokio::select! {
                _ = sleep(debounce) => {
                    if let Some(mut entry) = state.get_mut(&user_id) {
                        entry.status = PresenceStatus::Offline;
                        entry.session_count = 0;
                    }
                    pending.remove(&user_id);
                    broadcaster.broadcast(user_id, PresenceStatus::Offline).await;
                    tracing::debug!(user_id = %user_id, "User went offline (debounced)");
                }
                _ = cancel.notified() => {
                    tracing::debug!(user_id = %user_id, "Offline timer cancelled (reconnected)");
                }
            }
        });
    }

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

    pub fn get_status(&self, user_id: &UserId) -> PresenceStatus {
        self.state
            .get(user_id)
            .map(|p| p.status)
            .unwrap_or(PresenceStatus::Offline)
    }
}