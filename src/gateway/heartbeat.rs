//! Heartbeat Monitor — finds and kills dead connections.
//!
//! This solves Problem #7: zombie connections eating memory.
//!
//! When someone closes their laptop lid, the WebSocket doesn't
//! send a disconnect signal — it just goes silent. Without a
//! heartbeat check, that session stays in memory forever. The
//! server thinks the user is still online. Over time, thousands
//! of zombie sessions pile up wasting RAM.
//!
//! The fix: this background task runs every 10 seconds and checks
//! every session's last_heartbeat timestamp (an AtomicU64 — no lock
//! needed). If a session hasn't sent any message in 60 seconds,
//! we reap it: remove from DashMap, free memory immediately (Rust
//! drops the struct — no GC needed), and trigger presence update.
//!
//! This task also chains into Problem #4: when we reap a session
//! and the user has no remaining sessions, we call
//! on_all_sessions_closed() which starts the 5-second debounce.

use crate::gateway::session::SessionStore;
use crate::presence::tracker::PresenceTracker;
use std::sync::Arc;
use std::time::Duration;

pub fn spawn_heartbeat_monitor(
    sessions: Arc<SessionStore>,
    presence: Arc<PresenceTracker>,
    check_interval: Duration,
    timeout: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(check_interval);
        loop {
            interval.tick().await;

            // Find all sessions that have gone silent.
            // This reads each session's AtomicU64 timestamp — no locks.
            let stale = sessions.find_stale_sessions(timeout.as_secs());

            if !stale.is_empty() {
                tracing::info!(count = stale.len(), "Reaping stale sessions");
            }

            for (user_id, session_id) in stale {
                tracing::debug!(
                    user_id = %user_id,
                    session_id = %session_id,
                    "Heartbeat timeout — disconnecting"
                );

                // Remove from DashMap. In Rust, this drops the Session
                // struct immediately — the mpsc sender closes, the
                // WebSocket task detects the closed channel and stops,
                // and all memory is freed right now. Not 2 minutes later
                // when a GC would get around to it.
                sessions.remove(user_id, session_id);

                // If this was the user's last session, trigger the
                // debounced offline flow from presence/tracker.rs.
                if !sessions.is_connected(&user_id) {
                    presence.on_all_sessions_closed(user_id).await;
                }
            }
        }
    })
}
