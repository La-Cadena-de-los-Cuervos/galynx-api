# syntax=docker/dockerfile:1

FROM rust:1.93-bookworm AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --bin galynx-api

FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/galynx-api /usr/local/bin/galynx-api

ENV PORT=3000
EXPOSE 3000

CMD ["galynx-api"]
