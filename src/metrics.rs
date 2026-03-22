use axum::{routing::get, Router};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;

/// Install the Prometheus metrics recorder and return a handle
/// that can render the /metrics endpoint.
pub fn install_recorder() -> metrics_exporter_prometheus::PrometheusHandle {
    let builder = PrometheusBuilder::new();
    builder
        .install_recorder()
        .expect("Failed to install Prometheus recorder")
}

/// Spawn a standalone metrics HTTP server on `addr`.
pub async fn serve_metrics(addr: SocketAddr, handle: metrics_exporter_prometheus::PrometheusHandle) {
    let app = Router::new().route(
        "/metrics",
        get(move || {
            let h = handle.clone();
            async move { h.render() }
        }),
    );

    tracing::info!(addr = %addr, "Metrics server listening");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind metrics server");
    axum::serve(listener, app).await.ok();
}

/// Register descriptive metadata for all custom metrics.
pub fn describe_metrics() {
    metrics::describe_counter!(
        "messages_routed_total",
        "Total messages routed through the gateway"
    );
    metrics::describe_histogram!(
        "message_fanout_duration_seconds",
        "Time to fan out a message to all recipients"
    );
    metrics::describe_histogram!(
        "fanout_recipients",
        "Number of recipients per message fan-out"
    );
    metrics::describe_gauge!(
        "active_connections",
        "Current number of active WebSocket connections"
    );
    metrics::describe_counter!(
        "redis_messages_published",
        "Messages published to Redis pub/sub"
    );
    metrics::describe_counter!(
        "redis_messages_received",
        "Messages received from Redis pub/sub"
    );
    metrics::describe_counter!(
        "presence_updates_broadcast",
        "Presence changes broadcast to users"
    );
    metrics::describe_histogram!(
        "presence_fanout_targets",
        "Number of users notified per presence change"
    );
    metrics::describe_counter!(
        "readstate_flushed_total",
        "Read state entries flushed to database"
    );
    metrics::describe_gauge!(
        "readstate_pending",
        "Read state entries pending flush"
    );
    metrics::describe_counter!(
        "rate_limit_rejected",
        "Requests rejected by rate limiter"
    );
}
