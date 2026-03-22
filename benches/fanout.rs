//! Benchmarks for hot-path operations.
//!
//! Run: cargo bench

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;

type UserId = Uuid;
type ChannelId = Uuid;
type MessageId = Uuid;

/// Benchmark: DashMap insert/update (simulates read state cache writes)
fn bench_readstate_cache(c: &mut Criterion) {
    let map: DashMap<(UserId, ChannelId), MessageId> = DashMap::new();

    // Pre-populate with some data
    let users: Vec<UserId> = (0..1000).map(|_| Uuid::new_v4()).collect();
    let channels: Vec<ChannelId> = (0..100).map(|_| Uuid::new_v4()).collect();

    let mut group = c.benchmark_group("readstate_cache");

    group.bench_function("insert_new", |b| {
        b.iter(|| {
            let uid = users[rand::random::<usize>() % users.len()];
            let cid = channels[rand::random::<usize>() % channels.len()];
            let mid = Uuid::new_v4();
            map.insert(black_box((uid, cid)), black_box(mid));
        });
    });

    group.bench_function("update_existing", |b| {
        // Pre-fill
        for u in &users {
            for c in &channels {
                map.insert((*u, *c), Uuid::new_v4());
            }
        }
        b.iter(|| {
            let uid = users[rand::random::<usize>() % users.len()];
            let cid = channels[rand::random::<usize>() % channels.len()];
            let mid = Uuid::new_v4();
            map.entry(black_box((uid, cid)))
                .and_modify(|existing| {
                    if mid > *existing {
                        *existing = mid;
                    }
                })
                .or_insert(mid);
        });
    });

    group.finish();
}

/// Benchmark: Session lookup and fan-out (simulates message delivery)
fn bench_session_fanout(c: &mut Criterion) {
    use tokio::sync::mpsc;

    let sessions: DashMap<UserId, Vec<mpsc::UnboundedSender<String>>> = DashMap::new();

    // Simulate 1000 users with 1-3 sessions each
    let user_ids: Vec<UserId> = (0..1000).map(|_| Uuid::new_v4()).collect();
    let mut receivers = Vec::new();

    for uid in &user_ids {
        let num_sessions = (rand::random::<usize>() % 3) + 1;
        let mut senders = Vec::new();
        for _ in 0..num_sessions {
            let (tx, rx) = mpsc::unbounded_channel();
            senders.push(tx);
            receivers.push(rx);
        }
        sessions.insert(*uid, senders);
    }

    let mut group = c.benchmark_group("session_fanout");

    // Benchmark fan-out to N users
    for n in [10, 50, 100, 500] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            let targets: Vec<UserId> = user_ids[..n].to_vec();
            let msg = "test message payload".to_string();
            b.iter(|| {
                for uid in &targets {
                    if let Some(senders) = sessions.get(uid) {
                        for tx in senders.iter() {
                            let _ = tx.send(black_box(msg.clone()));
                        }
                    }
                }
            });
        });
    }

    group.finish();
}

/// Benchmark: Token bucket rate limiter
fn bench_rate_limiter(c: &mut Criterion) {
    let buckets: DashMap<UserId, (f64, std::time::Instant)> = DashMap::new();
    let users: Vec<UserId> = (0..100).map(|_| Uuid::new_v4()).collect();

    c.bench_function("rate_limiter_check", |b| {
        b.iter(|| {
            let uid = users[rand::random::<usize>() % users.len()];
            let mut entry = buckets.entry(black_box(uid)).or_insert_with(|| {
                (10.0, std::time::Instant::now())
            });
            let (tokens, last) = entry.value_mut();
            let elapsed = last.elapsed().as_secs_f64();
            *tokens = (*tokens + elapsed * 5.0).min(10.0);
            *last = std::time::Instant::now();
            if *tokens >= 1.0 {
                *tokens -= 1.0;
                true
            } else {
                false
            }
        });
    });
}

criterion_group!(
    benches,
    bench_readstate_cache,
    bench_session_fanout,
    bench_rate_limiter,
);
criterion_main!(benches);
