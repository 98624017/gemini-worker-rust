# syntax=docker/dockerfile:1.7

FROM rust:1-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 appuser

WORKDIR /app

COPY --from=builder /app/target/release/rust-sync-proxy /usr/local/bin/rust-sync-proxy

ENV PORT=8787
ENV RUST_LOG=info
ENV MALLOC_CONF=background_thread:true,dirty_decay_ms:500,muzzy_decay_ms:500

EXPOSE 8787

USER appuser

ENTRYPOINT ["/usr/local/bin/rust-sync-proxy"]
