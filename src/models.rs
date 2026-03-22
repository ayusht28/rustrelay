use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

// ─── ID type aliases ────────────────────────────────────────────

pub type UserId = Uuid;
pub type ChannelId = Uuid;
pub type GuildId = Uuid;
pub type MessageId = Uuid;
pub type SessionId = Uuid;

// ─── Presence ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresenceStatus {
    Online,
    Idle,
    Dnd,
    Offline,
}

impl std::fmt::Display for PresenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Online => write!(f, "online"),
            Self::Idle => write!(f, "idle"),
            Self::Dnd => write!(f, "dnd"),
            Self::Offline => write!(f, "offline"),
        }
    }
}

// ─── Database models ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub token: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Guild {
    pub id: GuildId,
    pub name: String,
    pub owner_id: UserId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Channel {
    pub id: ChannelId,
    pub guild_id: GuildId,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Message {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub author_id: UserId,
    pub content: String,
    pub edited_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

// ─── Client → Server (incoming ops) ────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "op", content = "d")]
#[serde(rename_all = "snake_case")]
pub enum ClientMessage {
    /// Send a chat message to a channel
    SendMessage {
        channel_id: ChannelId,
        content: String,
    },
    /// Acknowledge reading up to a message in a channel
    AckMessage {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    /// Update own presence status
    UpdatePresence {
        status: PresenceStatus,
    },
    /// Heartbeat to keep connection alive
    Heartbeat {
        seq: u64,
    },
    /// Start typing indicator
    StartTyping {
        channel_id: ChannelId,
    },
}

// ─── Server → Client (outgoing events) ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", content = "d")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerEvent {
    /// Server is ready, includes session info
    Ready {
        session_id: SessionId,
        user: UserInfo,
        guilds: Vec<GuildInfo>,
        read_states: Vec<ReadState>,
        heartbeat_interval_ms: u64,
    },
    /// A new message was created
    MessageCreate(MessagePayload),
    /// A message was edited
    MessageUpdate(MessagePayload),
    /// A message was deleted
    MessageDelete {
        id: MessageId,
        channel_id: ChannelId,
    },
    /// A user's presence changed
    PresenceUpdate {
        user_id: UserId,
        status: PresenceStatus,
    },
    /// A user started typing
    TypingStart {
        user_id: UserId,
        channel_id: ChannelId,
        timestamp: DateTime<Utc>,
    },
    /// Heartbeat acknowledgement
    HeartbeatAck {
        seq: u64,
    },
}

/// Lightweight user info sent in READY payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: UserId,
    pub username: String,
}

/// Guild info sent in READY payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuildInfo {
    pub id: GuildId,
    pub name: String,
    pub channels: Vec<ChannelInfo>,
    pub member_count: i64,
}

/// Channel info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub id: ChannelId,
    pub name: String,
}

/// Message payload for create/update events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub author_id: UserId,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
}

/// Read state for a single channel
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ReadState {
    pub channel_id: ChannelId,
    pub last_read_message_id: MessageId,
}

// ─── Internal inter-node messages (via Redis) ───────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossNodePayload {
    pub source_node: String,
    pub target_user_ids: Vec<UserId>,
    pub event: Arc<str>, // JSON-serialized ServerEvent
}
