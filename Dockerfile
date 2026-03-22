# ── Build stage ──────────────────────────────────────────────────
FROM rust:1.77-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
# Create a dummy src to cache dependency builds
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>/dev/null || true

COPY src/ src/
COPY migrations/ migrations/
RUN cargo build --release

# ── Runtime stage ────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/rustrelay /usr/local/bin/rustrelay
COPY migrations/ /app/migrations/

WORKDIR /app
ENV RUST_LOG=rustrelay=info

EXPOSE 8080 9090

CMD ["rustrelay"]
