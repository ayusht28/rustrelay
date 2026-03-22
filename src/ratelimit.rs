use crate::models::UserId;
use dashmap::DashMap;
use std::time::Instant;

/// Per-user token bucket rate limiter.
///
/// Built from scratch (per the stretch goal spec) — no external crate.
///
/// Each user gets a bucket that fills at `refill_rate` tokens/sec
/// up to `capacity`. Every operation consumes 1 token. If the bucket
/// is empty, the request is rejected.
pub struct RateLimiter {
    buckets: DashMap<UserId, Bucket>,
    capacity: u32,
    refill_rate: f64, // tokens per second
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// - `capacity`: max tokens a user can accumulate
    /// - `refill_rate`: tokens added per second
    pub fn new(capacity: u32, refill_rate: f64) -> Self {
        Self {
            buckets: DashMap::new(),
            capacity,
            refill_rate,
        }
    }

    /// Try to consume one token. Returns `true` if allowed, `false` if rate limited.
    pub fn try_acquire(&self, user_id: &UserId) -> bool {
        let mut entry = self.buckets.entry(*user_id).or_insert_with(|| Bucket {
            tokens: self.capacity as f64,
            last_refill: Instant::now(),
        });

        let bucket = entry.value_mut();
        let now = Instant::now();
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();

        // Refill tokens based on elapsed time
        bucket.tokens = (bucket.tokens + elapsed * self.refill_rate).min(self.capacity as f64);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            metrics::counter!("rate_limit_rejected").increment(1);
            false
        }
    }

    /// Spawn periodic cleanup of stale buckets (users who disconnected long ago).
    pub fn spawn_cleanup(self: &std::sync::Arc<Self>) -> tokio::task::JoinHandle<()> {
        let limiter = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                let threshold = Instant::now() - std::time::Duration::from_secs(600);
                limiter
                    .buckets
                    .retain(|_, bucket| bucket.last_refill > threshold);
            }
        })
    }
}
