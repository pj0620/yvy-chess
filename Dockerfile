# syntax=docker/dockerfile:1

# 1. Web bundle
FROM oven/bun:1 AS web
WORKDIR /web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web/ .
RUN bun run build

# 2. Server binary. rust-toolchain.toml is deliberately not copied: the image's own stable
#    toolchain is used, so no extra toolchain download happens inside the build.
FROM rust:1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p yvy-server && cp target/release/yvy-server /usr/local/bin/yvy-server

# 3. Runtime: Debian trixie ships Stockfish 17 as a multi-arch package (installed at
#    /usr/games/stockfish). reqwest and sqlx use rustls, so no libssl is needed.
FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends stockfish ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=server /usr/local/bin/yvy-server /usr/local/bin/yvy-server
COPY --from=web /web/dist /srv/web
ENV STOCKFISH_PATH=/usr/games/stockfish \
    YVY_WEB_DIR=/srv/web \
    YVY_BIND=0.0.0.0:8080 \
    RUST_LOG=info
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s CMD ["yvy-server", "health"]
CMD ["yvy-server", "serve"]
