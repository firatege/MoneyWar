# ── Stage 1: Rust build ─────────────────────────────────────────────────────
FROM rust:1.82-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release -p moneywar-cli

# ── Stage 2: Runtime ─────────────────────────────────────────────────────────
FROM node:22-bookworm-slim

RUN apt-get update && apt-get install -y \
    python3 make g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Session server bağımlılıkları
COPY session-server/package.json ./session-server/
RUN cd session-server && npm install

COPY session-server/server.js ./session-server/

# Rust binary
COPY --from=builder /app/target/release/moneywar-cli /usr/local/bin/moneywar-cli

RUN mkdir -p /app/debug

EXPOSE 8080

CMD ["node", "session-server/server.js"]
