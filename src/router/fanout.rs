//! Message Router — the brain of message delivery.
//!
//! This solves Problem #3: hitting the database on every message.
//!
//! When Alice sends "Hey everyone!" to #general with 100 members,
//! we need to know who those 100 members are. Without caching,
//! that's a SQL query on every single message. At 1000 messages/sec,
//! the database gets hammered with 1000 SELECT queries just for lookups.
//!
//! The fix: cache the member list in a DashMap with a 5-minute TTL.
//! First message → DB query → cache. Next 999 messages → instant
//! cache hit (0.001ms). After 5 minutes, the cache expires and we
//! refresh. 1000 DB queries become 1.
//!
//! The router also handles cross-node delivery. If a channel member
//! is connected to a different server, we publish to Redis and that
//! server delivers locally. See redis_bridge.rs for details.

use crate::db;
use crate::gateway::session::SessionStore;
use crate::models::*;
use crate::router::redis_bridge::RedisBridge;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Cached member list with an expiry timestamp.
struct CachedMembers {
    members: Vec<UserId>,
    fetched_at: Instant,
}

pub struct MessageRouter {
    pool: sqlx::PgPool,
    sessions: Arc<SessionStore>,
    redis: Arc<RedisBridge>,
    node_id: String,

    /// Channel member cache. Key = channel_id, value = member list + timestamp.
    /// DashMap for the same reason as everywhere else — 64 shards, no contention.
    member_cache: DashMap<ChannelId, CachedMembers>,

    /// How long a cached member list stays valid.
    cache_ttl: Duration,
}

impl MessageRouter {
    pub fn new(
        pool: sqlx::PgPool,
        sessions: Arc<SessionStore>,
        redis: Arc<RedisBridge>,
        node_id: String,
        cache_ttl: Duration,
    ) -> Self {
        Self {
            pool,
            sessions,
            redis,
            node_id,
            member_cache: DashMap::new(),
            cache_ttl,
        }
    }

    /// Handle a send_message operation. The full pipeline:
    ///
    ///   1. Save the message to PostgreSQL (persistence).
    ///   2. Look up channel members (cached — usually 0.001ms).
    ///   3. For each member on THIS server, push into their mpsc channel.
    ///   4. For members on OTHER servers, publish to Redis.
    ///
    /// Total time: typically 2-5ms. No GC pause can interrupt this.
    pub async fn handle_send(
        &self,
        author_id: UserId,
        channel_id: ChannelId,
        content: &str,
    ) -> anyhow::Result<()> {
        // Step 1: Persist to database.
        let message = db::insert_message(&self.pool, channel_id, author_id, content).await?;

        let event = ServerEvent::MessageCreate(MessagePayload {
            id: message.id,
            channel_id: message.channel_id,
            author_id: message.author_id,
            content: message.content.clone(),
            timestamp: message.created_at,
            edited_at: None,
        });

        // Step 2: Who's in this channel? (Cached — see get_channel_members below.)
        let members = self.get_channel_members(channel_id).await?;

        // Step 3 & 4: Deliver to local sessions, collect remote targets.
        let mut remote_targets = Vec::new();
        for &member_id in &members {
            if self.sessions.is_connected(&member_id) {
                // This user is on OUR server. Push into their mpsc channel.
                // Takes ~0.001ms. No lock, no network, no waiting.
                self.sessions.send_to_user(&member_id, &event);
            } else {
                // This user is NOT on our server. They might be on
                // another server node. We'll tell Redis about them.
                remote_targets.push(member_id);
            }
        }

        // Step 4: Publish to Redis for cross-node delivery.
        // Other servers are subscribed and will deliver locally.
        if !remote_targets.is_empty() {
            let event_json: Arc<str> = serde_json::to_string(&event)?.into();
            let payload = CrossNodePayload {
                source_node: self.node_id.clone(),
                target_user_ids: remote_targets,
                event: event_json,
            };
            self.redis.publish_event(payload).await?;
        }

        metrics::counter!("messages_routed_total").increment(1);
        metrics::histogram!("fanout_recipients").record(members.len() as f64);

        Ok(())
    }

    /// Get channel members — from cache if fresh, from DB if stale.
    ///
    /// First call for a channel: DB query (~5ms) → result cached.
    /// Subsequent calls within the TTL: instant cache hit (~0.001ms).
    /// After TTL expires: re-fetch from DB.
    pub async fn get_channel_members(
        &self,
        channel_id: ChannelId,
    ) -> anyhow::Result<Vec<UserId>> {
        // Try the cache first.
        if let Some(entry) = self.member_cache.get(&channel_id) {
            if entry.fetched_at.elapsed() < self.cache_ttl {
                return Ok(entry.members.clone()); // Cache hit!
            }
        }

        // Cache miss or expired — go to the database.
        let members = db::get_channel_member_ids(&self.pool, channel_id).await?;
        self.member_cache.insert(
            channel_id,
            CachedMembers {
                members: members.clone(),
                fetched_at: Instant::now(),
            },
        );

        Ok(members)
    }

    /// Force-invalidate the cache for a channel.
    /// Call this when someone joins or leaves a channel so the next
    /// message picks up the new member list.
    pub fn invalidate_channel_cache(&self, channel_id: &ChannelId) {
        self.member_cache.remove(channel_id);
    }

    /// Background task that cleans up expired cache entries every 60 seconds.
    pub fn spawn_cache_cleanup(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let router = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let ttl = router.cache_ttl;
                router
                    .member_cache
                    .retain(|_, v| v.fetched_at.elapsed() < ttl);
            }
        })
    }
}
