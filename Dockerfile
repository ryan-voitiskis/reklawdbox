# syntax=docker/dockerfile:1
# reklawdbox as a network MCP server (Streamable HTTP) for Linux hosts and
# clusters. Usage and the snapshot workflow: docs/linux.md ("Container").
#
# Base images are pinned by the digest of their multi-arch index.

FROM rust:1-trixie@sha256:8a4e90c7946b74626488b1446f712c0fe0ed1a596cbd3d871684bc9c5433ae07 AS build
WORKDIR /src
COPY . .
# SQLCipher builds a vendored OpenSSL: the full rust image already has perl/make/cc.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin reklawdbox \
    && cp target/release/reklawdbox /reklawdbox

FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --user-group --home-dir /home/reklawdbox --create-home reklawdbox
COPY --from=build /reklawdbox /usr/local/bin/reklawdbox
COPY docker/entrypoint.sh /usr/local/bin/reklawdbox-entrypoint
COPY docker/no-backup.sh /usr/local/lib/reklawdbox/no-backup.sh
ENV REKLAWDBOX_HTTP_ADDR=0.0.0.0:8000 \
    REKORDBOX_DB_PATH=/home/reklawdbox/rekordbox/master.db \
    REKLAWDBOX_BACKUP_SCRIPT=/usr/local/lib/reklawdbox/no-backup.sh \
    RUST_LOG=info
USER 10001
WORKDIR /home/reklawdbox
EXPOSE 8000
ENTRYPOINT ["/usr/local/bin/reklawdbox-entrypoint"]
