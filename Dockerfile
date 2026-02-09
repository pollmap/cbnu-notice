# ── Stage 1: Build ──
FROM rust:1.84-slim AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY tests/ tests/
RUN cargo build --release && strip target/release/cbnu-notice-bot

# ── Stage 2: Runtime ──
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/cbnu-notice-bot .
COPY config.toml .
RUN mkdir -p /data
ENV DATABASE_PATH=/data/notices.db
CMD ["./cbnu-notice-bot", "serve"]
