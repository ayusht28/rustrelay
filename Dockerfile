# ── Build stage ──────────────────────────────────────────────────
FROM rust:1.82-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
# Dummy source to cache dependency compilation
RUN mkdir -p src && echo "fn main() {}" > src/main.rs && \
    echo "" > src/lib.rs
RUN cargo build --release 2>/dev/null || true

# Copy real source and force rebuild
COPY src/ src/
COPY migrations/ migrations/
RUN touch src/main.rs src/lib.rs && cargo build --release

# ── Runtime stage ────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/rustrelay /usr/local/bin/rustrelay
COPY migrations/ /app/migrations/

WORKDIR /app
ENV RUST_LOG=rustrelay=info

# Render routes external traffic to port 10000 (set via PORT env var)
EXPOSE 10000

CMD ["rustrelay"]
