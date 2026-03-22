//! RustRelay — Entry Point
//!
//! This file wires all the subsystems together:
//!   - Connects to PostgreSQL and Redis
//!   - Creates the session store, message router, presence tracker, read state cache
//!   - Spawns background tasks (heartbeat monitor, read state flusher, cache cleanup)
//!   - Starts the HTTP + WebSocket server
//!   - Handles graceful shutdown (flush buffers, drain connections)

use rustrelay::auth::Auth;
use rustrelay::config::Config;
use rustrelay::gateway::handler::AppState;
use rustrelay::gateway::heartbeat;
use rustrelay::gateway::session::SessionStore;
use rustrelay::metrics as app_metrics;
use rustrelay::presence::broadcast::PresenceBroadcaster;
use rustrelay::presence::tracker::PresenceTracker;
use rustrelay::readstate::cache::ReadStateCache;
use rustrelay::router::fanout::MessageRouter;
use rustrelay::router::redis_bridge::{self, RedisBridge};
use rustrelay::routes;

use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Load configuration from .env ────────────────────────
    dotenvy::dotenv().ok();
    let config = Arc::new(Config::from_env()?);

    // ── Set up structured logging ───────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rustrelay=info,tower_http=info".into()),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();

    tracing::info!(node_id = %config.node_id, "Starting RustRelay gateway");

    // ── Prometheus metrics ──────────────────────────────────
    let prom_handle = app_metrics::install_recorder();
    app_metrics::describe_metrics();

    // ── Connect to PostgreSQL ───────────────────────────────
    let pool = PgPoolOptions::new()
        .max_connections(50)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;
    tracing::info!("Connected to PostgreSQL");

    // Run database migrations (creates tables if they don't exist).
    sqlx::migrate!("./migrations").run(&pool).await.ok();

    // ── Connect to Redis ────────────────────────────────────
    let redis = Arc::new(
        RedisBridge::new(&config.redis_url, config.node_id.clone()).await?,
    );

    // ── Create the session store (Problem #2) ───────────────
    // DashMap with 64 shards. All sessions live here.
    let sessions = Arc::new(SessionStore::new());

    // ── Create the presence system (Problems #4, #5) ────────
    // Broadcaster batches updates every 100ms.
    // Tracker debounces offline by 5 seconds.
    let broadcaster = PresenceBroadcaster::new(
        pool.clone(),
        sessions.clone(),
        redis.clone(),
        Duration::from_millis(100),
    );
    let presence = Arc::new(PresenceTracker::new(
        broadcaster,
        config.presence_offline_debounce,
    ));

    // ── Create the read state cache (Problem #6) ────────────
    // Buffers acks in memory, flushes to DB every 5 seconds.
    let read_states = Arc::new(ReadStateCache::new());
    read_states.spawn_flusher(
        pool.clone(),
        config.readstate_flush_interval,
        config.readstate_flush_batch_size,
    );

    // ── Create the message router (Problem #3) ──────────────
    // Caches channel members with a 5-minute TTL.
    let router = Arc::new(MessageRouter::new(
        pool.clone(),
        sessions.clone(),
        redis.clone(),
        config.node_id.clone(),
        config.channel_member_cache_ttl,
    ));
    router.spawn_cache_cleanup();

    // ── Start Redis subscriber for cross-node messages ──────
    // This listens for events published by other server nodes
    // and delivers them to users connected to THIS node.
    redis_bridge::spawn_subscriber(
        &config.redis_url,
        config.node_id.clone(),
        sessions.clone(),
    )
    .await?;

    // ── Start the heartbeat monitor (Problem #7) ────────────
    // Checks every 10 seconds, reaps sessions silent for 60 seconds.
    heartbeat::spawn_heartbeat_monitor(
        sessions.clone(),
        presence.clone(),
        Duration::from_secs(10),
        config.heartbeat_timeout,
    );

    // ── Build the application state ─────────────────────────
    let state = AppState {
        config: config.clone(),
        pool: pool.clone(),
        auth: Arc::new(Auth::new(&config.jwt_secret)),
        sessions: sessions.clone(),
        router,
        presence,
        read_states: read_states.clone(),
    };

    // ── Build HTTP router ───────────────────────────────────
    let app = routes::build_router(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // ── Start the Prometheus metrics server (port 9090) ─────
    let metrics_addr = config.metrics_addr();
    tokio::spawn(app_metrics::serve_metrics(metrics_addr, prom_handle));

    // ── Start the main HTTP + WebSocket server ──────────────
    let listen_addr = config.listen_addr();
    tracing::info!(addr = %listen_addr, "Gateway server listening");

    let listener = tokio::net::TcpListener::bind(listen_addr).await?;

    // Graceful shutdown: wait for Ctrl+C, then clean up.
    let shutdown = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
        tracing::info!("Shutdown signal received");
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;

    // ── Shutdown: flush everything before exiting ────────────
    tracing::info!("Draining connections and flushing buffers...");
    read_states.flush_now(&pool).await?; // Don't lose buffered read states
    pool.close().await;                   // Close database connections
    tracing::info!("Shutdown complete");

    Ok(())
}
