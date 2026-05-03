use crate::gateway::session::SessionStore;
use crate::models::*;

use fred::prelude::*;
use std::sync::Arc;

/// Handles Redis pub/sub for cross-node event delivery.
///
/// When REDIS_URL is empty, the bridge runs in **disabled** mode:
/// all publish/subscribe operations become silent no-ops, so the
/// server works as a single-node deployment without Redis.
pub struct RedisBridge {
    publisher: Option<RedisClient>,
    node_id: String,
    disabled: bool,
}

impl RedisBridge {
    /// Connect to Redis and return a live bridge.
    pub async fn new(redis_url: &str, node_id: String) -> anyhow::Result<Self> {
        let config = RedisConfig::from_url(redis_url)?;
        let publisher = RedisClient::new(config, None, None, None);
        publisher.connect();
        publisher.wait_for_connect().await?;

        tracing::info!(node_id = %node_id, "Redis publisher connected");

        Ok(Self {
            publisher: Some(publisher),
            node_id,
            disabled: false,
        })
    }

    /// Create a no-op bridge — used when REDIS_URL is empty.
    /// All publish/set/get calls succeed instantly without doing anything.
    pub fn new_disabled(node_id: String) -> Self {
        tracing::info!(
            node_id = %node_id,
            "Redis disabled — running in single-node mode (no cross-node delivery)"
        );
        Self {
            publisher: None,
            node_id,
            disabled: true,
        }
    }

    /// Publish an event for cross-node delivery.
    pub async fn publish_event(&self, payload: CrossNodePayload) -> anyhow::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let publisher = self.publisher.as_ref().unwrap();
        let json = serde_json::to_string(&payload)?;
        let _: () = publisher
            .publish("rustrelay:events", json.as_str())
            .await?;
        metrics::counter!("redis_messages_published").increment(1);
        Ok(())
    }

    /// Publish a presence update across all nodes.
    pub async fn publish_presence(
        &self,
        user_id: UserId,
        status: PresenceStatus,
    ) -> anyhow::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let publisher = self.publisher.as_ref().unwrap();
        let payload = serde_json::json!({
            "source_node": self.node_id,
            "user_id": user_id,
            "status": status,
        });
        let _: () = publisher
            .publish("rustrelay:presence", payload.to_string().as_str())
            .await?;
        Ok(())
    }

    /// Store a presence key in Redis with TTL.
    pub async fn set_presence(
        &self,
        user_id: UserId,
        status: PresenceStatus,
        ttl_secs: i64,
    ) -> anyhow::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let publisher = self.publisher.as_ref().unwrap();
        let key = format!("presence:{user_id}");
        let value = serde_json::json!({
            "status": status,
            "node": self.node_id,
            "updated_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();

        let _: () = publisher
            .set(
                key.as_str(),
                value.as_str(),
                Some(Expiration::EX(ttl_secs)),
                None,
                false,
            )
            .await?;
        Ok(())
    }

    /// Get a user's presence from Redis.
    pub async fn get_presence(&self, user_id: UserId) -> anyhow::Result<Option<String>> {
        if self.disabled {
            return Ok(None);
        }
        let publisher = self.publisher.as_ref().unwrap();
        let key = format!("presence:{user_id}");
        let result: Option<String> = publisher.get(key.as_str()).await?;
        Ok(result)
    }

    /// Remove a user's presence key.
    pub async fn remove_presence(&self, user_id: UserId) -> anyhow::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let publisher = self.publisher.as_ref().unwrap();
        let key = format!("presence:{user_id}");
        let _: () = publisher.del(key.as_str()).await?;
        Ok(())
    }

    /// Get the node_id for this bridge.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }
}

/// Spawn a subscriber task that listens on `rustrelay:events` and
/// delivers messages to local sessions.
/// If redis_url is empty, returns immediately (no-op).
pub async fn spawn_subscriber(
    redis_url: &str,
    node_id: String,
    sessions: Arc<SessionStore>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    if redis_url.is_empty() {
        tracing::info!(node_id = %node_id, "Redis subscriber disabled");
        return Ok(tokio::spawn(async {}));
    }

    let config = RedisConfig::from_url(redis_url)?;
    let subscriber = RedisClient::new(config, None, None, None);
    subscriber.connect();
    subscriber.wait_for_connect().await?;

    let mut message_stream = subscriber.message_rx();

    // Subscribe to the events channel
    let _: () = subscriber.subscribe("rustrelay:events").await?;
    let _: () = subscriber.subscribe("rustrelay:presence").await?;

    tracing::info!(node_id = %node_id, "Redis subscriber listening");

    let handle = tokio::spawn(async move {
        while let Ok(message) = message_stream.recv().await {
            let channel = message.channel.to_string();
            let payload = match message.value.as_str() {
                Some(s) => s.to_string(),
                None => continue,
            };

            match channel.as_str() {
                "rustrelay:events" => {
                    handle_cross_node_event(&sessions, &node_id, &payload);
                }
                "rustrelay:presence" => {
                    handle_cross_node_presence(&sessions, &node_id, &payload);
                }
                _ => {}
            }
        }
        tracing::warn!("Redis subscriber stream ended");
    });

    Ok(handle)
}

fn handle_cross_node_event(sessions: &SessionStore, my_node_id: &str, raw: &str) {
    let payload: CrossNodePayload = match serde_json::from_str(raw) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "Invalid cross-node payload");
            return;
        }
    };

    // Skip events we published ourselves
    if payload.source_node == my_node_id {
        return;
    }

    // Parse the event JSON
    let event: ServerEvent = match serde_json::from_str(&payload.event) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "Invalid event in cross-node payload");
            return;
        }
    };

    // Deliver to local sessions
    let mut delivered = 0;
    for user_id in &payload.target_user_ids {
        delivered += sessions.send_to_user(user_id, &event);
    }

    metrics::counter!("redis_messages_received").increment(1);
    tracing::trace!(
        source = %payload.source_node,
        targets = payload.target_user_ids.len(),
        delivered,
        "Cross-node event delivered"
    );
}

fn handle_cross_node_presence(sessions: &SessionStore, my_node_id: &str, raw: &str) {
    #[derive(serde::Deserialize)]
    struct PresencePayload {
        source_node: String,
        user_id: UserId,
        status: PresenceStatus,
    }

    let payload: PresencePayload = match serde_json::from_str(raw) {
        Ok(p) => p,
        Err(_) => return,
    };

    if payload.source_node == my_node_id {
        return;
    }

    let event = ServerEvent::PresenceUpdate {
        user_id: payload.user_id,
        status: payload.status,
    };

    // Broadcast to all local sessions (they'll filter by relevance client-side,
    // or we could add guild-based filtering here for efficiency)
    for uid in sessions.connected_user_ids() {
        sessions.send_to_user(&uid, &event);
    }
}
