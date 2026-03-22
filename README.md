# RustRelay

**A real-time messaging backend built in Rust — modeled after the system Discord rebuilt when Go couldn't keep up.**

---

## The story behind this project

Discord runs one of the largest real-time messaging platforms on the planet. Millions of users, billions of messages. One of their most critical services — the one that tracks which messages you've read — was originally written in Go.

It worked. Mostly.

The problem was Go's garbage collector. Every couple of minutes, Go would stop everything to clean up unused memory. During that pause, messages would pile up, users would see delays, and the whole system would hiccup. Discord tried tuning it. They tried workarounds. Nothing fixed the fundamental issue: Go pauses to clean up, and you can't turn that off.

So they rewrote it in Rust. Rust doesn't have a garbage collector. Memory gets freed the moment it's no longer needed — the compiler figures this out before the code even runs. The result was dramatic. Latency dropped. The hiccups disappeared. They needed fewer servers to handle the same load.

This project is my attempt to rebuild that system from scratch. Not just the read-state service they rewrote, but the full real-time backend: WebSocket connections, message delivery, presence tracking, and the read-state buffering system that started it all.

---

## What problems does this solve?

I didn't build this just to write Rust code. Each part of the system exists because there's a real engineering problem behind it.

### 1. Garbage collection freezes everything

In Go, the GC pauses the entire server every ~2 minutes to scan and free memory. At scale, that means every connected user feels a stutter at the same time. There's no way around it — it's baked into how Go manages memory.

Rust doesn't have this problem. There's no GC. When a message finishes being delivered, the memory is freed right there, instantly. No pile-up, no scanning, no pauses.

**Where in the code:** This isn't one file — it's a property of the entire codebase. Every function, every struct, every temporary variable gets cleaned up the moment it goes out of scope.

### 2. Thousands of users fighting for one lock

A regular HashMap with a lock means everyone waits in line. User #3847 wants to read, but user #12 is writing, so 3846 users just sit there.

I used DashMap — it splits the data into 64 independent segments. Alice's connection lives on segment #12, Bob's on segment #47. They never block each other.

**Where in the code:** `src/gateway/session.rs`

### 3. Hitting the database on every message

When someone sends a message to a channel with 100 members, I need to know who those 100 members are. Querying the database every single time is wasteful — at 1000 messages per second, that's 1000 identical queries.

So I cache the member list. First message triggers a DB query. The next 999 hit the cache. It expires after 5 minutes and refreshes.

**Where in the code:** `src/router/fanout.rs`

### 4. Presence status flickers on bad connections

Your phone goes through a tunnel. WiFi drops for 2 seconds. Without handling this, everyone in your servers sees "offline" then "online" — two broadcasts to potentially 50,000 users for a 2-second blip.

I added a 5-second debounce. When someone disconnects, I start a timer. If they come back within 5 seconds, the timer cancels silently. Nobody gets notified. If they don't come back, then I broadcast offline.

**Where in the code:** `src/presence/tracker.rs`

### 5. One status change triggers 50,000 notifications

If you're in 50 servers with 1000 members each, going online means notifying 50,000 people. If 10 users come online in the same second, that's half a million notifications.

I batch these. Every 100 milliseconds, I collect all the status changes that happened, remove duplicates (if you share 3 servers with someone, they get notified once, not three times), and send everything in one burst.

**Where in the code:** `src/presence/broadcast.rs`

### 6. Millions of database writes per second

Every time you open a channel, your app says "I've read up to message #X." At scale, that's millions of these acknowledgments per second. Writing each one to the database individually would melt it.

This is the exact service Discord rewrote. My approach: buffer everything in memory (takes microseconds), then flush to the database in one bulk query every 5 seconds. 50,000 individual writes become a handful of batch inserts.

The flush runs on a completely separate task. It never blocks message delivery. Think of it like a restaurant — the waiter keeps serving tables while the dishwasher collects plates in the back every few minutes.

**Where in the code:** `src/readstate/cache.rs`

### 7. Dead connections eating memory forever

Someone closes their laptop without disconnecting. Their session just goes silent. Without cleanup, thousands of these zombie sessions pile up over time, wasting memory and confusing the presence system.

A background task checks every 10 seconds. If a connection hasn't sent anything in 60 seconds, it gets reaped. In Rust, the memory is freed the instant the session is removed — no waiting for a GC cycle.

**Where in the code:** `src/gateway/heartbeat.rs`

---

## Demo — see it working

Here's what it looks like when you run the system and connect two users.

### Starting the server

```
$ cargo run

Starting RustRelay gateway node_id="node-1"
Connected to PostgreSQL
Redis publisher connected node_id="node-1"
Redis subscriber listening node_id="node-1"
Metrics server listening addr=0.0.0.0:9090
Gateway server listening addr=0.0.0.0:8080
```

### Alice connects

```
$ websocat ws://127.0.0.1:8080/ws
token_alice

{"t":"READY","d":{
  "session_id":"f04e6fbf-...",
  "user":{"id":"00000000-...0001","username":"alice"},
  "guilds":[{
    "name":"RustRelay Dev",
    "channels":[
      {"name":"general"},
      {"name":"random"}
    ],
    "member_count":4
  }],
  "heartbeat_interval_ms":30000
}}
```

### Alice sends a message — Bob receives it instantly

Alice types:

```
{"op":"send_message","d":{"channel_id":"20000000-...0001","content":"Hey Bob!"}}
```

Bob's terminal (separate connection) immediately shows:

```
{"t":"MESSAGE_CREATE","d":{
  "content":"Hey Bob!",
  "author_id":"00000000-...0001",
  "channel_id":"20000000-...0001",
  "timestamp":"2026-03-22T19:22:10Z"
}}
```

Server log confirms:

```
Message routed latency_ms=3 channel_id=20000000-...
```

3 milliseconds. No GC spike. Every single message, consistently.

### Bob disconnects — presence debounce kicks in

Bob closes his terminal. Five seconds of silence pass. Then Alice sees:

```
{"t":"PRESENCE_UPDATE","d":{"user_id":"00000000-...0002","status":"offline"}}
```

That 5-second gap is the debounce. If Bob had reconnected within those 5 seconds, Alice would have seen nothing — zero wasted traffic.

### Heartbeat keeps connections alive

```
{"op":"heartbeat","d":{"seq":1}}

{"t":"HEARTBEAT_ACK","d":{"seq":1}}
```

If a client stops sending heartbeats for 60 seconds, the server reaps the connection and frees the memory instantly.

### Server stats

```
$ curl http://localhost:8080/api/stats

{
  "node_id": "node-1",
  "active_connections": 2,
  "readstate_pending": 0,
  "readstate_total_ops": 5
}
```

### Prometheus metrics

```
$ curl http://localhost:9090/metrics

messages_routed_total 12
active_connections 2
message_fanout_duration_seconds_bucket{le="0.005"} 11
presence_updates_broadcast 3
readstate_flushed_total 5
```

---

## What's in the box

```
src/
├── main.rs                 Wires everything together
├── config.rs               Reads settings from .env
├── models.rs               Data types + WebSocket protocol
├── auth.rs                 JWT + simple token auth
├── db.rs                   All PostgreSQL queries
├── error.rs                Error handling
├── routes.rs               REST API + WebSocket upgrade
├── ratelimit.rs            Token bucket rate limiter (hand-built)
├── metrics.rs              Prometheus metrics
│
├── gateway/
│   ├── session.rs          Session store — DashMap + mpsc channels
│   ├── handler.rs          WebSocket lifecycle
│   └── heartbeat.rs        Reaps dead connections
│
├── router/
│   ├── fanout.rs           Message delivery + member cache
│   └── redis_bridge.rs     Cross-server delivery via Redis
│
├── presence/
│   ├── tracker.rs          Debounced offline detection
│   └── broadcast.rs        Batched status notifications
│
└── readstate/
    └── cache.rs            Write buffer + bulk flush
```

---

## How a message travels through the code

```
Alice sends "Hey everyone!"
       │
       ▼
gateway/handler.rs       → receives WebSocket message, parses it
       │
       ▼
router/fanout.rs         → the brain of delivery
       ├── db.rs             save to PostgreSQL
       ├── DashMap cache     look up channel members (cached)
       ├── session.rs        push to each member's channel
       └── redis_bridge.rs   publish to Redis for other servers
```

---

## Requirements

| What             | Why                                             |
| ---------------- | ----------------------------------------------- |
| Rust 1.75+       | The language. Install from https://rustup.rs    |
| PostgreSQL 16    | Stores messages, users, guilds, channels        |
| Redis or Memurai | Pub/sub for cross-server messaging and presence |
| websocat         | Command-line WebSocket client for testing       |

### Installing on Windows

**Rust:**
Download and run `rustup-init.exe` from https://www.rust-lang.org/tools/install

**PostgreSQL:**
Download from https://www.enterprisedb.com/downloads/postgres-postgresql-downloads — pick version 16. During install, only check PostgreSQL Server and Command Line Tools.

**Redis:**
Download the MSI from https://github.com/tporadowski/redis/releases — install with defaults. Or install Memurai from https://www.memurai.com/get-memurai

**websocat:**

```
cargo install websocat
```

### Installing on Mac

```bash
brew install postgresql@16 redis
brew services start postgresql@16
brew services start redis
cargo install websocat
```

### Installing on Linux

```bash
sudo apt install postgresql redis-server -y
sudo systemctl start postgresql redis-server
cargo install websocat
```

---

## Setting it up

### Database setup

Connect to PostgreSQL and create the database:

```sql
CREATE USER rustrelay WITH PASSWORD 'password';
CREATE DATABASE rustrelay OWNER rustrelay;
```

Load the tables:

Windows:

```powershell
& "C:\Program Files\PostgreSQL\16\bin\psql.exe" -U rustrelay -d rustrelay -f migrations\001_initial.sql
```

Mac/Linux:

```bash
psql -U rustrelay -d rustrelay -f migrations/001_initial.sql
```

### Configuration

```bash
cp .env.example .env
```

Open `.env` and set:

```
DATABASE_URL=postgres://rustrelay:password@localhost:5432/rustrelay
REDIS_URL=redis://localhost:6379
JWT_SECRET=anyrandomstringhere
```

### Running

```
cargo run
```

First build takes a few minutes. After that it's instant. You'll see 6 startup lines ending with `Gateway server listening`.

---

## Test users

Comes pre-loaded with 4 users in one guild with 2 channels:

| User    | Token           |
| ------- | --------------- |
| alice   | `token_alice`   |
| bob     | `token_bob`     |
| charlie | `token_charlie` |
| dave    | `token_dave`    |

Channels: `#general` and `#random`

---

## WebSocket commands

**Send a message:**

```json
{
  "op": "send_message",
  "d": {
    "channel_id": "20000000-0000-0000-0000-000000000001",
    "content": "Hello!"
  }
}
```

**Mark as read:**

```json
{
  "op": "ack_message",
  "d": {
    "channel_id": "20000000-0000-0000-0000-000000000001",
    "message_id": "..."
  }
}
```

**Heartbeat:**

```json
{ "op": "heartbeat", "d": { "seq": 1 } }
```

**Set status:**

```json
{ "op": "update_presence", "d": { "status": "dnd" } }
```

**Typing indicator:**

```json
{
  "op": "start_typing",
  "d": { "channel_id": "20000000-0000-0000-0000-000000000001" }
}
```

---

## REST API

| Method | Path                         | What it does       |
| ------ | ---------------------------- | ------------------ |
| GET    | `/api/health`                | Health check       |
| POST   | `/api/login`                 | Get a JWT token    |
| GET    | `/api/guilds/:id/channels`   | List channels      |
| GET    | `/api/channels/:id/messages` | Get messages       |
| POST   | `/api/channels/:id/messages` | Send a message     |
| GET    | `/api/stats`                 | Server statistics  |
| GET    | `:9090/metrics`              | Prometheus metrics |

---

## Running multiple servers

```bash
NODE_ID=node-1 PORT=8080 cargo run    # terminal 1
NODE_ID=node-2 PORT=8081 cargo run    # terminal 2
```

Alice connects to node-1, Bob connects to node-2. When Alice sends a message, node-1 publishes to Redis, node-2 picks it up and delivers to Bob. The servers don't need to know about each other.

---

## Tech stack

| Tool    | Purpose                       |
| ------- | ----------------------------- |
| Tokio   | Async runtime                 |
| axum    | HTTP + WebSocket server       |
| DashMap | Lock-free concurrent hash map |
| sqlx    | Async PostgreSQL              |
| redis   | Pub/sub for scaling           |
| serde   | JSON handling                 |
| tracing | Structured logging            |
| metrics | Prometheus integration        |

---

## License

MIT
