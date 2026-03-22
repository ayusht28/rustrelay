//! Read State Cache — the exact service Discord rewrote from Go to Rust.
//!
//! This solves Problem #6: millions of database writes per second.
//!
//! Every time a user opens a channel, their client says "I've read up
//! to message X." At scale, that's millions of acknowledgments per second.
//! Writing each one individually to PostgreSQL would kill the database.
//!
//! The solution: buffer everything in a DashMap (takes ~0.001ms per write),
//! then flush to the database in one bulk query every 5 seconds.
//!
//! Key insight: if the same user reads the same channel 10 times in
//! 5 seconds, the DashMap key `(user_id, channel_id)` just overwrites.
//! 10 writes coalesce into 1.
//!
//! The flush runs on a SEPARATE async task (tokio::spawn). It never
//! blocks message delivery. See the question "doesn't flushing cause
//! lag?" — the answer is no, because Tokio tasks run independently.

use crate::db;
use crate::models::*;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct ReadStateCache {
    /// The buffer: (user_id, channel_id) → last message_id they've read.
    ///
    /// DashMap is used here for the same reason as in session.rs —
    /// 64 shards means thousands of concurrent read-ack writers
    /// don't block each other.
    pending: DashMap<(UserId, ChannelId), MessageId>,

    /// How many updates are sitting in the buffer right now.
    pending_count: AtomicU64,

    /// Lifetime counter — total acks processed since server start.
    total_ops: AtomicU64,
}

impl ReadStateCache {
    pub fn new() -> Self {
        Self {
            pending: DashMap::new(),
            pending_count: AtomicU64::new(0),
            total_ops: AtomicU64::new(0),
        }
    }

    /// Record a read-state update. THIS IS THE HOT PATH.
    ///
    /// Called potentially thousands of times per second. Must be fast.
    /// All it does is write to a DashMap — no database, no network,
    /// no lock contention. ~0.001ms per call.
    ///
    /// If the same (user, channel) pair already has an entry, we only
    /// update if the new message_id is later. This means rapid reads
    /// on the same channel naturally coalesce.
    pub fn update(&self, user_id: UserId, channel_id: ChannelId, message_id: MessageId) {
        self.pending
            .entry((user_id, channel_id))
            .and_modify(|existing| {
                // Only update if the new message is later than what we have.
                // This prevents stale acks from overwriting fresher ones.
                if message_id > *existing {
                    *existing = message_id;
                }
            })
            .or_insert(message_id);

        self.pending_count.fetch_add(1, Ordering::Relaxed);
        self.total_ops.fetch_add(1, Ordering::Relaxed);
    }

    /// Drain all pending updates into a Vec for flushing.
    /// After this, the DashMap is empty and ready for new writes.
    fn drain(&self) -> Vec<(UserId, ChannelId, MessageId)> {
        let keys: Vec<(UserId, ChannelId)> =
            self.pending.iter().map(|entry| *entry.key()).collect();

        let mut batch = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some((k, v)) = self.pending.remove(&key) {
                batch.push((k.0, k.1, v));
            }
        }

        self.pending_count.store(0, Ordering::Relaxed);
        batch
    }

    /// How many updates are buffered right now.
    pub fn pending_count(&self) -> u64 {
        self.pending_count.load(Ordering::Relaxed)
    }

    /// Total acks processed since server start.
    pub fn total_ops(&self) -> u64 {
        self.total_ops.load(Ordering::Relaxed)
    }

    /// Spawn the background flusher — a separate async task.
    ///
    /// This is the key design: the flusher runs on its OWN tokio task.
    /// It wakes up every `interval` seconds, drains the buffer, and
    /// sends one bulk SQL query. While it's talking to the database,
    /// the message delivery tasks keep running — they write to the
    /// DashMap without waiting for the flush to finish.
    ///
    /// Think of it like a restaurant: the waiter (message delivery)
    /// keeps serving tables, while the dishwasher (flusher) collects
    /// dirty plates every 5 minutes and washes them in one batch.
    /// The waiter never stops serving because the dishwasher is busy.
    pub fn spawn_flusher(
        self: &Arc<Self>,
        pool: sqlx::PgPool,
        interval: Duration,
        batch_threshold: usize,
    ) -> tokio::task::JoinHandle<()> {
        let cache = Arc::clone(self);

        tokio::spawn(async move {
            let mut timer = tokio::time::interval(interval);
            loop {
                timer.tick().await;

                let count = cache.pending_count() as usize;
                if count == 0 {
                    continue; // Nothing to flush, go back to sleep.
                }

                let batch = cache.drain();
                if batch.is_empty() {
                    continue;
                }

                let batch_len = batch.len();
                tracing::debug!(batch_size = batch_len, "Flushing read states to DB");

                // Split into sub-batches if the batch is very large.
                for chunk in batch.chunks(batch_threshold.max(100)) {
                    match db::upsert_read_states_batch(&pool, chunk).await {
                        Ok(()) => {
                            metrics::counter!("readstate_flushed_total")
                                .increment(chunk.len() as u64);
                        }
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                batch_size = chunk.len(),
                                "Failed to flush read states — re-buffering"
                            );
                            // If the DB write fails, put the items back in the buffer.
                            // They'll be retried on the next flush cycle.
                            for &(user_id, channel_id, message_id) in chunk {
                                cache.update(user_id, channel_id, message_id);
                            }
                        }
                    }
                }

                metrics::gauge!("readstate_pending").set(cache.pending_count() as f64);
            }
        })
    }

    /// Force an immediate flush. Called during graceful shutdown so we
    /// don't lose any buffered data when the server stops.
    pub async fn flush_now(&self, pool: &sqlx::PgPool) -> anyhow::Result<()> {
        let batch = self.drain();
        if !batch.is_empty() {
            tracing::info!(count = batch.len(), "Flushing read states on shutdown");
            db::upsert_read_states_batch(pool, &batch).await?;
        }
        Ok(())
    }
}

impl Default for ReadStateCache {
    fn default() -> Self {
        Self::new()
    }
}
