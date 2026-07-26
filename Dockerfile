# ── Stage 1: Rust build ──────────────────────────────────────────────────────
#
# Cargo.toml'daki `rust-version` (MSRV, şu an 1.88) bu sürümün **altına**
# inemez. 1.86'dayken sessiz bir tuzaktı: let-chain sözdizimi (1.88'de
# stabil) lokalde derleniyor ama deploy anında image build'inde patlıyordu.
FROM rust:1.96-bookworm AS rust-builder

WORKDIR /app
COPY . .
RUN cargo build --release -p moneywar-web

# ── Stage 2: Frontend build ───────────────────────────────────────────────────
FROM node:22-bookworm-slim AS frontend-builder

WORKDIR /app/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --ignore-scripts
COPY web/ ./
RUN npm run build

# ── Stage 3: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=rust-builder /app/target/release/moneywar-web /usr/local/bin/moneywar-web
COPY --from=frontend-builder /app/web/dist /app/web/dist

RUN mkdir -p /app/debug

EXPOSE 8080

ENV RUST_LOG=info

CMD ["/usr/local/bin/moneywar-web"]
