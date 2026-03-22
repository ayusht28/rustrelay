# RustRelay

A real-time message routing and presence system built in Rust — inspired by Discord's
decision to rewrite their Read States service from Go to Rust.

This isn't a toy project. It mirrors the actual architecture behind Discord's real-time
infrastructure and solves the exact same problems they faced at scale.

---

## Why does this project exist?

Discord's engineering team had a service written in Go that tracked which messages each
user had read. It worked fine most of the time, but every ~2 minutes Go's garbage collector
would freeze the entire service to clean up memory. At Discord's scale (millions of users),
those freezes caused latency spikes that affected everyone.

They rewrote the service in Rust. The result: zero GC pauses, lower memory usage, better
tail latency, fewer servers needed.

This project recreates that architecture. It solves 7 specific problems:

| # | Problem | Our solution | File |
|---|---------|-------------|------|
| 1 | Go's GC freezes all messages every ~2 min | Rust has no GC — memory freed instantly via ownership | Entire codebase |
| 2 | Thousands of users fighting for one lock | DashMap (64 shards) + mpsc channels per session | `gateway/session.rs` |
| 3 | Database query on every single message | Channel member cache with 5-min TTL | `router/fanout.rs` |
| 4 | Presence flickers when phone loses signal | 5-second debounced offline detection | `presence/tracker.rs` |
| 5 | One status change notifies 50,000 users | Batched + de-duplicated presence broadcasts | `presence/broadcast.rs` |
| 6 | Millions of read-state writes per second | In-memory buffer with periodic bulk flush | `readstate/cache.rs` |
| 7 | Dead connections wasting memory forever | Heartbeat monitor reaps stale sessions | `gateway/heartbeat.rs` |

---

## What's inside

**WebSocket Gateway** — Users connect over WebSockets. Each user can be on multiple
devices (phone + laptop) at the same time. The gateway authenticates them, manages their
sessions, and handles heartbeats to detect dead connections.

**Message Router** — When someone sends a message, the router saves it to PostgreSQL,
looks up who's in that channel (cached), and delivers it to every recipient's WebSocket.
If a recipient is on a different server, it publishes to Redis for cross-node delivery.

**Presence Tracker** — Tracks who's online, idle, or offline. Uses a 5-second debounce
so that brief disconnections (phone going through a tunnel) don't cause flickering. Batches
presence updates to avoid overwhelming the system when many users change status at once.

**Read State Service** — Tracks the last message each user has read in each channel. This
is the exact service Discord rewrote. Instead of writing to the database on every read
acknowledgment, it buffers updates in memory and flushes them in bulk every 5 seconds.

---

## Prerequisites

You need 3 things installed:

### 1. Rust

**Windows:** Download from https://www.rust-lang.org/tools/install — run `rustup-init.exe`.

**Mac/Linux:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Restart your terminal after installing, then verify:

```
rustc --version
cargo --version
```

### 2. PostgreSQL

**Windows:** Download from https://www.enterprisedb.com/downloads/postgres-postgresql-downloads

Pick version 16 for Windows. During installation:
- Set password to `password` (or anything you'll remember)
- Keep port as `5432`
- Only check "PostgreSQL Server" and "Command Line Tools"
- Uncheck pgAdmin and Stack Builder (you don't need them)
- When Stack Builder pops up after install, click Cancel

**Mac:**
```bash
brew install postgresql@16
brew services start postgresql@16
```

**Linux:**
```bash
sudo apt install postgresql postgresql-client -y
sudo systemctl start postgresql
```

### 3. Redis (or Memurai on Windows)

**Windows:** Redis doesn't run on Windows natively. Install Memurai instead — it's a
drop-in replacement that works exactly the same:

Download from https://www.memurai.com/get-memurai — install with all defaults.
It starts automatically on port 6379.

**Mac:**
```bash
brew install redis
brew services start redis
```

**Linux:**
```bash
sudo apt install redis-server -y
sudo systemctl start redis-server
```

### 4. websocat (for testing)

```
cargo install websocat
```

Takes a few minutes to compile. This gives you a command-line WebSocket client.

---

## Setup

### Option A: Automatic setup (Windows)

Open a terminal in the project folder and run:

```
scripts\setup_windows.bat
```

This creates the database, user, tables, and test data automatically.
Then skip ahead to "Configure and Run" below.

### Option B: Manual setup (all platforms)

**Step 1: Create the database and user.**

Windows (adjust the path to where you installed PostgreSQL):
```powershell
& "C:\Program Files\PostgreSQL\16\bin\psql.exe" -U postgres
```

Mac/Linux:
```bash
psql -U postgres
```

Type your PostgreSQL password, then run these 3 commands:

```sql
CREATE USER rustrelay WITH PASSWORD 'password';
CREATE DATABASE rustrelay OWNER rustrelay;
\q
```

**Step 2: Load the tables and test data.**

Windows:
```powershell
& "C:\Program Files\PostgreSQL\16\bin\psql.exe" -U rustrelay -d rustrelay -f migrations\001_initial.sql
```

Mac/Linux:
```bash
psql -U rustrelay -d rustrelay -f migrations/001_initial.sql
```

Type `password` when asked. If you see the INSERT statements run with no errors, the
database is ready.

### Option C: Docker (if you have it)

If you have Docker installed, this is the quickest way:

```bash
docker compose up -d
```

This starts PostgreSQL and Redis with all tables and test data pre-loaded.
Wait 10 seconds, then check both show "healthy":

```bash
docker compose ps
```

---

## Configure and Run

**Step 1: Create the config file.**

Windows:
```powershell
copy .env.example .env
```

Mac/Linux:
```bash
cp .env.example .env
```

**Step 2: Edit `.env` in VS Code or any editor.**

Change this line:
```
JWT_SECRET=change-me-to-a-long-random-string
```
To:
```
JWT_SECRET=mysecret123
```

If you installed PostgreSQL with a different password, also update:
```
DATABASE_URL=postgres://rustrelay:YOUR_PASSWORD_HERE@localhost:5432/rustrelay
```

Save the file.

**Step 3: Run the server.**

```
cargo run
```

First build takes 2-5 minutes (downloading dependencies). When ready, you'll see:

```
Starting RustRelay gateway node_id="node-1"
Connected to PostgreSQL
Redis publisher connected
Redis subscriber listening
Metrics server listening addr=0.0.0.0:9090
Gateway server listening addr=0.0.0.0:8080
```

All 6 lines = everything is working. Keep this terminal open.

**Step 4: Verify.**

Open a new terminal:

```
curl http://localhost:8080/api/health
```

Should print `OK`.

```
curl http://localhost:8080/api/stats
```

Should print JSON with `node_id`, `active_connections`, etc.

---

## Testing it out

### Connect as Alice

Open a new terminal:

```
websocat ws://127.0.0.1:8080/ws
```

Cursor blinks. Type and press Enter:

```
token_alice
```

You'll receive a `READY` event with Alice's session info, guilds, and channels.

### Connect as Bob

Open another terminal:

```
websocat ws://127.0.0.1:8080/ws
```

Type:

```
token_bob
```

### Send a message

In Alice's terminal, type:

```json
{"op":"send_message","d":{"channel_id":"20000000-0000-0000-0000-000000000001","content":"Hey Bob!"}}
```

**What to check:**
- Alice sees a `MESSAGE_CREATE` event (she gets her own message back)
- Bob sees the same `MESSAGE_CREATE` event (message was delivered to him)
- Server logs show `Message routed latency_ms=3` (delivered in ~3ms, no GC pause)

### Test heartbeat

In any connected terminal:

```json
{"op":"heartbeat","d":{"seq":1}}
```

You'll get back `HEARTBEAT_ACK` with the same seq number.

### Test presence

Close Bob's terminal (Ctrl+C). Wait 5 seconds. Alice receives:

```json
{"t":"PRESENCE_UPDATE","d":{"user_id":"...","status":"offline"}}
```

The 5-second wait is the debounce. Reconnect Bob and Alice sees `"status":"online"`.

### Test typing indicator

In Alice's terminal:

```json
{"op":"start_typing","d":{"channel_id":"20000000-0000-0000-0000-000000000001"}}
```

Bob sees `TYPING_START`. Alice doesn't see it (you don't see your own typing).

---

## Verification checklist

```
[ ] cargo run shows all 6 startup lines
[ ] curl /api/health returns "OK"
[ ] curl /api/stats returns JSON
[ ] Alice connects and gets READY
[ ] Bob connects and gets READY
[ ] Alice sends message → both receive it
[ ] Server log shows latency_ms under 10
[ ] Heartbeat returns HEARTBEAT_ACK
[ ] Close Bob → Alice sees offline after 5s
[ ] Reopen Bob → Alice sees online
[ ] curl localhost:9090/metrics returns Prometheus data
```

---

## Available test users

| Username | Token | User ID |
|----------|-------|---------|
| alice | `token_alice` | `00000000-...-000000000001` |
| bob | `token_bob` | `00000000-...-000000000002` |
| charlie | `token_charlie` | `00000000-...-000000000003` |
| dave | `token_dave` | `00000000-...-000000000004` |

All 4 are in the "RustRelay Dev" guild with 2 channels:
- `#general` — `20000000-0000-0000-0000-000000000001`
- `#random` — `20000000-0000-0000-0000-000000000002`

---

## WebSocket protocol

### What the client sends

| Operation | Example |
|-----------|---------|
| Send a message | `{"op":"send_message","d":{"channel_id":"...","content":"Hello!"}}` |
| Mark as read | `{"op":"ack_message","d":{"channel_id":"...","message_id":"..."}}` |
| Update status | `{"op":"update_presence","d":{"status":"dnd"}}` |
| Keep alive | `{"op":"heartbeat","d":{"seq":1}}` |
| Typing | `{"op":"start_typing","d":{"channel_id":"..."}}` |

### What the server sends

| Event | When |
|-------|------|
| `READY` | Right after authentication |
| `MESSAGE_CREATE` | Someone sent a message in a channel you're in |
| `MESSAGE_UPDATE` | A message was edited |
| `MESSAGE_DELETE` | A message was deleted |
| `PRESENCE_UPDATE` | A user in your guild went online/offline/idle/dnd |
| `TYPING_START` | Someone started typing |
| `HEARTBEAT_ACK` | Server acknowledges your heartbeat |

---

## REST API

| Method | Path | What it does |
|--------|------|-------------|
| GET | `/api/health` | Returns "OK" if the server is running |
| POST | `/api/login` | Get a JWT token |
| GET | `/api/guilds/:id/channels` | List channels in a guild |
| GET | `/api/channels/:id/messages` | Get recent messages |
| POST | `/api/channels/:id/messages` | Send a message via REST |
| GET | `/api/stats` | Server statistics |
| GET | `:9090/metrics` | Prometheus metrics |

---

## Project structure

```
rustrelay/
├── src/
│   ├── main.rs                 Entry point — wires everything together
│   ├── lib.rs                  Module declarations
│   ├── config.rs               Reads settings from .env file
│   ├── models.rs               All data types + WebSocket protocol
│   ├── auth.rs                 JWT tokens + simple token auth for dev
│   ├── db.rs                   Every SQL query lives here
│   ├── error.rs                Error types with proper HTTP status codes
│   ├── routes.rs               REST endpoints + WebSocket upgrade
│   ├── ratelimit.rs            Token bucket rate limiter (hand-built)
│   ├── metrics.rs              Prometheus metric definitions
│   │
│   ├── gateway/                 ← Problem 1, 2, 7
│   │   ├── session.rs          DashMap session store (64 shards, mpsc per user)
│   │   ├── handler.rs          WebSocket lifecycle
│   │   └── heartbeat.rs        Reaps dead sessions every 10 seconds
│   │
│   ├── router/                  ← Problem 3
│   │   ├── fanout.rs           Message routing with cached member lookup
│   │   └── redis_bridge.rs     Redis pub/sub for cross-server delivery
│   │
│   ├── presence/                ← Problem 4, 5
│   │   ├── tracker.rs          Debounced offline (5s timer with cancel)
│   │   └── broadcast.rs        Batched + de-duplicated fan-out
│   │
│   └── readstate/               ← Problem 6 (the Discord rewrite)
│       └── cache.rs            DashMap buffer → bulk flush every 5 seconds
│
├── migrations/
│   └── 001_initial.sql          Schema + test data
│
├── tests/
│   └── gateway_integration.rs   End-to-end WebSocket tests
│
├── benches/
│   └── fanout.rs                Performance benchmarks
│
├── scripts/
│   ├── setup_windows.bat        Automatic Windows setup (no Docker)
│   └── load_test.sh             Stress test with concurrent clients
│
├── docker-compose.yml           PostgreSQL + Redis (optional, if you have Docker)
├── Dockerfile                   Production build (optional)
├── Cargo.toml                   Dependencies
├── .env.example                 Configuration template
└── .gitignore
```

---

## How it all connects

When Alice sends "Hey everyone!":

```
Alice's browser
    │
    ▼
gateway/handler.rs          Receives WebSocket message, parses JSON
    │
    ▼
router/fanout.rs            The brain:
    ├──→ db.rs              Step 1: Save message to PostgreSQL
    ├──→ DashMap cache      Step 2: Look up channel members (cached)
    ├──→ session.rs         Step 3: Push to each member's mpsc channel
    └──→ redis_bridge.rs    Step 4: Publish to Redis for other servers
```

When Bob reads the message:

```
Bob sends ack_message
    │
    ▼
readstate/cache.rs          Writes to DashMap (0.001ms)
    │
    ... 5 seconds pass ...
    │
    ▼
db.rs                       One bulk INSERT for all buffered acks
```

When Charlie disconnects:

```
WebSocket closes
    │
    ▼
presence/tracker.rs         Starts 5-second debounce timer
    │
    ... 5 seconds, no reconnect ...
    │
    ▼
presence/broadcast.rs       Batches the update, broadcasts
```

---

## Running multiple servers

```bash
NODE_ID=node-1 PORT=8080 cargo run
NODE_ID=node-2 PORT=8081 cargo run
```

Both share state through Redis. Alice on node-1 can message Bob on node-2.

---

## Running tests

```
cargo check
cargo test
cargo bench
```

---

## Stopping everything

```
Ctrl+C in the cargo run terminal

Windows — stop PostgreSQL:
  net stop postgresql-x64-16

Windows — stop Memurai:
  net stop memurai

If using Docker instead:
  docker compose down
```

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| "Connection refused" on cargo run | PostgreSQL or Redis/Memurai isn't running |
| "password authentication failed" | Check DATABASE_URL in .env matches your PostgreSQL password |
| "Invalid token" on websocat | Type exactly `token_alice` — no quotes, no spaces |
| Build fails "linker not found" | Install Visual Studio Build Tools: https://visualstudio.microsoft.com/visual-cpp-build-tools/ |
| Port 8080 in use | Change `PORT=8081` in `.env` |
| "database does not exist" | Run the setup script or create manually (see Setup section) |
| psql not found | Use full path: `& "C:\Program Files\PostgreSQL\16\bin\psql.exe" -U postgres` |
| Memurai not starting | Open Services (Win+R → services.msc) → find Memurai → Start |

---

## Tech stack

| Tool | Why |
|------|-----|
| Tokio | Async runtime — thousands of tasks on a few threads |
| axum | Web framework with native WebSocket support |
| DashMap | Concurrent hash map — 64 shards, near-zero lock contention |
| sqlx | Async PostgreSQL with compile-time query checking |
| fred | Redis client with pub/sub support |
| serde | Fast JSON serialization |
| tracing | Structured logging |
| metrics | Prometheus-compatible tracking |

---

## License

MIT
