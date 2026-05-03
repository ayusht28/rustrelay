use std::env;
use std::net::SocketAddr;
use std::time::Duration;

/// Application configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub node_id: String,
    pub database_url: String,
    pub redis_url: String,
    pub jwt_secret: String,
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
    pub presence_offline_debounce: Duration,
    pub readstate_flush_interval: Duration,
    pub readstate_flush_batch_size: usize,
    pub channel_member_cache_ttl: Duration,
    pub metrics_port: u16,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()?,
            node_id: env::var("NODE_ID").unwrap_or_else(|_| {
                format!("node-{}", uuid::Uuid::new_v4().as_simple())
            }),
            database_url: env::var("DATABASE_URL")?,
            // Empty string = disabled (single-node mode, no cross-node pub/sub)
            redis_url: env::var("REDIS_URL").unwrap_or_default(),
            jwt_secret: env::var("JWT_SECRET")?,
            heartbeat_interval: Duration::from_secs(
                env::var("HEARTBEAT_INTERVAL_SECS")
                    .unwrap_or_else(|_| "30".into())
                    .parse()?,
            ),
            heartbeat_timeout: Duration::from_secs(
                env::var("HEARTBEAT_TIMEOUT_SECS")
                    .unwrap_or_else(|_| "60".into())
                    .parse()?,
            ),
            presence_offline_debounce: Duration::from_secs(
                env::var("PRESENCE_OFFLINE_DEBOUNCE_SECS")
                    .unwrap_or_else(|_| "5".into())
                    .parse()?,
            ),
            readstate_flush_interval: Duration::from_secs(
                env::var("READSTATE_FLUSH_INTERVAL_SECS")
                    .unwrap_or_else(|_| "5".into())
                    .parse()?,
            ),
            readstate_flush_batch_size: env::var("READSTATE_FLUSH_BATCH_SIZE")
                .unwrap_or_else(|_| "1000".into())
                .parse()?,
            channel_member_cache_ttl: Duration::from_secs(
                env::var("CHANNEL_MEMBER_CACHE_TTL_SECS")
                    .unwrap_or_else(|_| "300".into())
                    .parse()?,
            ),
            metrics_port: env::var("METRICS_PORT")
                .unwrap_or_else(|_| "9090".into())
                .parse()?,
        })
    }

    pub fn listen_addr(&self) -> SocketAddr {
        format!("{}:{}", self.host, self.port).parse().unwrap()
    }

    pub fn metrics_addr(&self) -> SocketAddr {
        format!("{}:{}", self.host, self.metrics_port).parse().unwrap()
    }
}
