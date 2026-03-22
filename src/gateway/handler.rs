//! WebSocket Connection Handler — the gateway's main logic.
//!
//! This file handles the full lifecycle of a WebSocket connection:
//!   1. Client connects → authenticate (JWT or dev token)
//!   2. Register a session in the SessionStore
//!   3. Update presence to online
//!   4. Send a READY event with guilds, channels, read states
//!   5. Enter the main loop: read client messages, dispatch to subsystems
//!   6. On disconnect: clean up session, update presence
//!
//! Every temporary object created during message processing is freed
//! the instant the function returns — Rust's ownership system guarantees
//! this at compile time. No garbage pile, no GC pause. (Problem #1)

use crate::auth;
use crate::config::Config;
use crate::db;
use crate::gateway::session::{Session, SessionStore};
use crate::models::*;
use crate::presence::tracker::PresenceTracker;
use crate::readstate::cache::ReadStateCache;
use crate::router::fanout::MessageRouter;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Shared application state — injected into every handler.
/// Each field is wrapped in Arc so it can be cheaply cloned
/// across async tasks without copying the underlying data.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pool: sqlx::PgPool,
    pub auth: Arc<auth::Auth>,
    pub sessions: Arc<SessionStore>,
    pub router: Arc<MessageRouter>,
    pub presence: Arc<PresenceTracker>,
    pub read_states: Arc<ReadStateCache>,
}

/// Handle a new WebSocket connection from start to finish.
pub async fn handle_connection(state: AppState, mut ws: WebSocket) {
    // ── Step 1: Authenticate ────────────────────────────────
    // The client must send their auth token as the very first message.
    // They have 5 seconds to do this or we close the connection.
    let user = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        ws.recv(),
    )
    .await
    {
        Ok(Some(Ok(WsMessage::Text(token)))) => {
            match auth::authenticate(&state.pool, &state.auth, &token).await {
                Ok(user) => user,
                Err(e) => {
                    let _ = ws
                        .send(WsMessage::Text(
                            serde_json::json!({"error": e.to_string()}).to_string().into(),
                        ))
                        .await;
                    let _ = ws.close().await;
                    return;
                }
            }
        }
        _ => {
            let _ = ws
                .send(WsMessage::Text(
                    serde_json::json!({"error": "Auth timeout — send your token within 5 seconds"})
                        .to_string()
                        .into(),
                ))
                .await;
            let _ = ws.close().await;
            return;
        }
    };

    let user_id = user.id;
    let username = user.username.clone();
    tracing::info!(user_id = %user_id, username = %username, "User authenticated");

    // ── Step 2: Create a session ────────────────────────────
    // Each session gets its own mpsc channel. The sender stays in
    // the SessionStore; the receiver is used below to forward events
    // to the WebSocket.
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ServerEvent>();
    let session = Session::new(user_id, username.clone(), event_tx);
    let session_id = session.session_id;
    let heartbeat_tracker = session.last_heartbeat.clone();
    state.sessions.insert(session);

    // ── Step 3: Update presence ─────────────────────────────
    state.presence.on_session_open(user_id).await;

    // ── Step 4: Send READY event ────────────────────────────
    let ready = build_ready_payload(&state, user_id, &username, session_id).await;
    let ready_json = serde_json::to_string(&ready).unwrap_or_default();
    if ws.send(WsMessage::Text(ready_json.into())).await.is_err() {
        cleanup(&state, user_id, session_id).await;
        return;
    }

    // ── Step 5: Split socket into reader + writer ───────────
    let (mut ws_sink, mut ws_stream) = ws.split();

    // Writer task: takes events from the mpsc channel and sends
    // them to the client's WebSocket. Runs on its own async task.
    let sink_handle = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let json = match serde_json::to_string(&event) {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to serialize event");
                    continue;
                }
            };
            if ws_sink.send(WsMessage::Text(json.into())).await.is_err() {
                break; // Client disconnected
            }
        }
    });

    // Reader loop: reads messages from the client and dispatches them.
    while let Some(msg) = ws_stream.next().await {
        match msg {
            Ok(WsMessage::Text(text)) => {
                // Every message from the client updates the heartbeat timestamp.
                // This is how the heartbeat monitor (Problem #7) knows the
                // connection is still alive.
                heartbeat_tracker.store(
                    chrono::Utc::now().timestamp() as u64,
                    std::sync::atomic::Ordering::Relaxed,
                );
                handle_client_message(&state, user_id, &text).await;
            }
            Ok(WsMessage::Ping(_)) => {
                // Axum auto-responds with Pong. Just update heartbeat.
                heartbeat_tracker.store(
                    chrono::Utc::now().timestamp() as u64,
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
            Ok(WsMessage::Close(_)) | Err(_) => break,
            _ => {}
        }
    }

    // ── Step 6: Client disconnected — clean up ──────────────
    sink_handle.abort();
    cleanup(&state, user_id, session_id).await;
}

/// Build the READY event — everything the client needs on connect.
async fn build_ready_payload(
    state: &AppState,
    user_id: UserId,
    username: &str,
    session_id: SessionId,
) -> ServerEvent {
    let guilds = db::get_user_guilds_info(&state.pool, user_id)
        .await
        .unwrap_or_default();

    let read_states = db::get_user_read_states(&state.pool, user_id)
        .await
        .unwrap_or_default();

    ServerEvent::Ready {
        session_id,
        user: UserInfo {
            id: user_id,
            username: username.to_string(),
        },
        guilds,
        read_states,
        heartbeat_interval_ms: state.config.heartbeat_interval.as_millis() as u64,
    }
}

/// Dispatch a client message to the appropriate subsystem.
async fn handle_client_message(state: &AppState, user_id: UserId, text: &str) {
    let msg: ClientMessage = match serde_json::from_str(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(user_id = %user_id, error = %e, "Invalid client message");
            return;
        }
    };

    match msg {
        // ── Send a chat message ─────────────────────────────
        // This is the hot path. It goes through the message router
        // which handles DB persistence, member lookup (cached), and
        // fan-out. See router/fanout.rs for the full pipeline.
        ClientMessage::SendMessage { channel_id, content } => {
            let started = std::time::Instant::now();
            if let Err(e) = state.router.handle_send(user_id, channel_id, &content).await {
                tracing::error!(error = %e, "Failed to route message");
            }
            let elapsed = started.elapsed();
            tracing::debug!(
                latency_ms = elapsed.as_millis(),
                channel_id = %channel_id,
                "Message routed"
            );
            metrics::histogram!("message_fanout_duration_seconds").record(elapsed.as_secs_f64());
        }

        // ── Mark a channel as read ──────────────────────────
        // This goes to the read state cache (Problem #6).
        // Just a DashMap write — 0.001ms. The background flusher
        // will batch-write to the DB later.
        ClientMessage::AckMessage { channel_id, message_id } => {
            state.read_states.update(user_id, channel_id, message_id);
        }

        // ── Update presence ─────────────────────────────────
        ClientMessage::UpdatePresence { status } => {
            state.presence.set_status(user_id, status).await;
        }

        // ── Heartbeat ───────────────────────────────────────
        ClientMessage::Heartbeat { seq } => {
            state
                .sessions
                .send_to_user(&user_id, &ServerEvent::HeartbeatAck { seq });
        }

        // ── Typing indicator ────────────────────────────────
        // Ephemeral — not saved to DB, just fanned out to channel
        // members. Auto-expires client-side after ~8 seconds.
        ClientMessage::StartTyping { channel_id } => {
            let event = ServerEvent::TypingStart {
                user_id,
                channel_id,
                timestamp: chrono::Utc::now(),
            };
            if let Ok(members) = state.router.get_channel_members(channel_id).await {
                let targets: Vec<_> = members.into_iter().filter(|id| *id != user_id).collect();
                state.sessions.send_to_users(&targets, &event);
            }
        }
    }
}

/// Clean up when a session disconnects.
async fn cleanup(state: &AppState, user_id: UserId, session_id: SessionId) {
    state.sessions.remove(user_id, session_id);
    tracing::info!(user_id = %user_id, session_id = %session_id, "Session disconnected");

    // If this was the user's last session, trigger debounced offline.
    if !state.sessions.is_connected(&user_id) {
        state.presence.on_all_sessions_closed(user_id).await;
    }

    metrics::gauge!("active_connections").set(state.sessions.total_connections() as f64);
}
