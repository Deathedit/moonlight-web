# syntax=docker/dockerfile:1
#
# Moonlight Web: Rust bridge + built web UI in one small image.
# Build from a checkout with submodules (git clone --recursive).

# ---- Web UI ----
FROM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# ---- Bridge ----
FROM rust:1-bookworm AS bridge
# cmake + OpenSSL for moonlight-common-c, libclang for bindgen.
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake clang libclang-dev pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY moonlight-common-sys moonlight-common-sys
COPY bridge bridge
RUN test -f moonlight-common-sys/moonlight-common-c/CMakeLists.txt \
    || (echo "moonlight-common-c submodule missing: clone with --recursive" && exit 1)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
    && cp target/release/moonlight-web /usr/local/bin/moonlight-web

# ---- Runtime ----
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends libssl3 ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 1000 --home-dir /data moonlight \
    && mkdir -p /data && chown moonlight /data

COPY --from=bridge /usr/local/bin/moonlight-web /usr/local/bin/moonlight-web
COPY --from=web /src/web/dist /app/web

ENV MLW_BIND=0.0.0.0:8080 \
    MLW_DATA_DIR=/data \
    MLW_WEB_DIR=/app/web \
    RUST_LOG=info

# Client identity, pairing certificates, host list and box art cache.
VOLUME /data
USER moonlight
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s CMD curl -fsS "http://127.0.0.1:${MLW_BIND##*:}/api/health" || exit 1

ENTRYPOINT ["moonlight-web"]
