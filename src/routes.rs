use crate::db;
use crate::error::{AppError, AppResult};
use crate::gateway::handler::{self, AppState};
use crate::models::*;

use axum::{
    extract::{Path, Query, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

/// Build the main application router.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // WebSocket gateway
        .route("/ws", get(ws_upgrade))
        // REST API
        .route("/api/health", get(health))
        .route("/api/login", post(login))
        .route("/api/guilds/:guild_id/channels", get(list_channels))
        .route(
            "/api/channels/:channel_id/messages",
            get(list_messages).post(send_message),
        )
        .route("/api/stats", get(stats))
        .with_state(state)
}

// ─── WebSocket upgrade ──────────────────────────────────────────

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handler::handle_connection(state, socket))
}

// ─── Health check ───────────────────────────────────────────────

async fn health() -> &'static str {
    "OK"
}

// ─── Simple login (returns a JWT) ───────────────────────────────

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    token: String,
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE username = $1 AND token = $2",
    )
    .bind(&req.username)
    .bind(&req.token)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Auth("Invalid credentials".into()))?;

    let jwt = state.auth.create_token(&user)?;

    Ok(Json(serde_json::json!({
        "token": jwt,
        "user": {
            "id": user.id,
            "username": user.username,
        }
    })))
}

// ─── List channels in a guild ───────────────────────────────────

async fn list_channels(
    State(state): State<AppState>,
    Path(guild_id): Path<GuildId>,
) -> AppResult<Json<Vec<ChannelInfo>>> {
    let channels = sqlx::query_as::<_, Channel>(
        "SELECT * FROM channels WHERE guild_id = $1 ORDER BY created_at",
    )
    .bind(guild_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(
        channels
            .into_iter()
            .map(|c| ChannelInfo {
                id: c.id,
                name: c.name,
            })
            .collect(),
    ))
}

// ─── List messages in a channel ─────────────────────────────────

#[derive(Deserialize)]
struct MessageQuery {
    limit: Option<i64>,
    before: Option<MessageId>,
}

async fn list_messages(
    State(state): State<AppState>,
    Path(channel_id): Path<ChannelId>,
    Query(query): Query<MessageQuery>,
) -> AppResult<Json<Vec<MessagePayload>>> {
    let limit = query.limit.unwrap_or(50).min(100);

    let messages = if let Some(before) = query.before {
        sqlx::query_as::<_, Message>(
            r#"
            SELECT * FROM messages
            WHERE channel_id = $1 AND created_at < (SELECT created_at FROM messages WHERE id = $2)
            ORDER BY created_at DESC
            LIMIT $3
            "#,
        )
        .bind(channel_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as::<_, Message>(
            "SELECT * FROM messages WHERE channel_id = $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(channel_id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    };

    Ok(Json(
        messages
            .into_iter()
            .map(|m| MessagePayload {
                id: m.id,
                channel_id: m.channel_id,
                author_id: m.author_id,
                content: m.content,
                timestamp: m.created_at,
                edited_at: m.edited_at,
            })
            .collect(),
    ))
}

// ─── Send a message via REST (alternative to WebSocket) ─────────

#[derive(Deserialize)]
struct SendMessageRequest {
    author_id: UserId,
    content: String,
}

async fn send_message(
    State(state): State<AppState>,
    Path(channel_id): Path<ChannelId>,
    Json(req): Json<SendMessageRequest>,
) -> AppResult<Json<MessagePayload>> {
    let message = db::insert_message(&state.pool, channel_id, req.author_id, &req.content)
        .await
        .map_err(AppError::Database)?;

    let payload = MessagePayload {
        id: message.id,
        channel_id: message.channel_id,
        author_id: message.author_id,
        content: message.content,
        timestamp: message.created_at,
        edited_at: None,
    };

    // Also route through the message router for real-time delivery
    if let Err(e) = state
        .router
        .handle_send(req.author_id, channel_id, &req.content)
        .await
    {
        tracing::error!(error = %e, "Failed to route REST message");
    }

    Ok(Json(payload))
}

// ─── Server stats ───────────────────────────────────────────────

async fn stats(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "node_id": state.config.node_id,
        "active_connections": state.sessions.total_connections(),
        "readstate_pending": state.read_states.pending_count(),
        "readstate_total_ops": state.read_states.total_ops(),
    }))
}
