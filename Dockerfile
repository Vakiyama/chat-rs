# ── build ────────────────────────────────────────────────
FROM rust:1-bookworm AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

COPY . .
RUN cargo build --release --bin server

# ── runtime ──────────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/server /usr/local/bin/server

# gRPC signaling
EXPOSE 3000/tcp
# WebRTC media (must match UDP_PORT)
EXPOSE 50000/udp

CMD ["server"]
