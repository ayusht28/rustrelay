//! Presence Broadcaster — batches and de-duplicates presence notifications.
//!
//! This solves Problem #5: one status change = 50,000 notifications.
//!
//! Charlie is in 50 guilds with 1000 members each. Going online
//! could mean notifying 50,000 users. If 10 users change status in
//! the same second, that's 500,000 notifications.
//!
//! The fix:
//!   1. Collect all presence changes into a batch.
//!   2. Every 100ms, flush the batch.
//!   3. De-duplicate: if Charlie went online→idle→online in 100ms,
//!      only send ONE "online" notification.
//!   4. De-duplicate targets: if Alice shares 3 guilds with Charlie,
//!      she gets notified once, not 3 times. The DB query
//!      get_colocated_user_ids() returns unique users.

use crate::db;
use crate::gateway::session::SessionStore;
use crate::models::*;
use crate::router::redis_bridge::RedisBridge;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

struct PendingUpdate {
    user_id: UserId,
    status: PresenceStatus,
}

pub struct PresenceBroadcaster {
    /// Channel to send updates to the background worker.
    tx: mpsc::UnboundedSender<PendingUpdate>,
}

impl PresenceBroadcaster {
    /// Create the broadcaster and spawn the background worker.
    pub fn new(
        pool: sqlx::PgPool,
        sessions: Arc<SessionStore>,
        redis: Arc<RedisBridge>,
        batch_interval: Duration,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let broadcaster = Arc::new(Self { tx });

        // The worker runs forever on its own task.
        tokio::spawn(broadcast_worker(pool, sessions, redis, rx, batch_interval));

        broadcaster
    }

    /// Enqueue a presence change. Cheap — just a channel send.
    pub async fn broadcast(&self, user_id: UserId, status: PresenceStatus) {
        let _ = self.tx.send(PendingUpdate { user_id, status });
    }
}

/// Background worker that collects updates and flushes them in batches.
///
/// Uses tokio::select! to either:
///   a) Receive a new update and add it to the batch, OR
///   b) Hit the 100ms timer and flush everything.
///
/// This means rapid status changes get coalesced. If 5 users go online
/// in 100ms, all 5 are processed together in one flush.
async fn broadcast_worker(
    pool: sqlx::PgPool,
    sessions: Arc<SessionStore>,
    redis: Arc<RedisBridge>,
    mut rx: mpsc::UnboundedReceiver<PendingUpdate>,
    batch_interval: Duration,
) {
    let mut batch: Vec<PendingUpdate> = Vec::new();
    let mut interval = tokio::time::interval(batch_interval);

    loop {
        tokio::select! {
            Some(update) = rx.recv() => {
                // De-duplicate within the batch: keep only the LATEST
                // status per user. If Charlie went online→idle→online
                // in 100ms, drop the first two — only "online" matters.
                batch.retain(|u| u.user_id != update.user_id);
                batch.push(update);

                // If the batch gets big, flush early to avoid buildup.
                if batch.len() >= 50 {
                    flush_batch(&pool, &sessions, &redis, &mut batch).await;
                }
            }
            _ = interval.tick() => {
                if !batch.is_empty() {
                    flush_batch(&pool, &sessions, &redis, &mut batch).await;
                }
            }
        }
    }
}

/// Process a batch of presence updates.
///
/// For each user who changed status:
///   1. Find all users who share a guild with them (DB query).
///      This returns a de-duplicated list — Alice in 3 of Charlie's
///      guilds appears only once.
///   2. Send to all local sessions.
///   3. Update Redis (for cross-node presence).
///   4. Publish to Redis pub/sub so other nodes can notify their users.
async fn flush_batch(
    pool: &sqlx::PgPool,
    sessions: &SessionStore,
    redis: &RedisBridge,
    batch: &mut Vec<PendingUpdate>,
) {
    let updates: Vec<PendingUpdate> = batch.drain(..).collect();

    for update in &updates {
        let event = ServerEvent::PresenceUpdate {
            user_id: update.user_id,
            status: update.status,
        };

        // get_colocated_user_ids returns UNIQUE users who share any
        // guild with this user. Alice in 3 shared guilds = 1 entry.
        match db::get_colocated_user_ids(pool, update.user_id).await {
            Ok(targets) => {
                // Deliver to local sessions.
                sessions.send_to_users(&targets, &event);

                // Update Redis presence store (with TTL for auto-expiry).
                if update.status == PresenceStatus::Offline {
                    let _ = redis.remove_presence(update.user_id).await;
                } else {
                    let _ = redis
                        .set_presence(update.user_id, update.status, 300)
                        .await;
                }

                // Publish to other nodes.
                let _ = redis
                    .publish_presence(update.user_id, update.status)
                    .await;

                metrics::counter!("presence_updates_broadcast").increment(1);
                metrics::histogram!("presence_fanout_targets").record(targets.len() as f64);
            }
            Err(e) => {
                tracing::error!(
                    user_id = %update.user_id,
                    error = %e,
                    "Failed to look up colocated users for presence broadcast"
                );
            }
        }
    }
}
