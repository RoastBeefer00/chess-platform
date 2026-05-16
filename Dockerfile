# syntax=docker/dockerfile:1.7

############################
# Builder
############################
FROM rust:1.90-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
        curl \
        clang \
        lld \
        perl \
        make \
        g++ \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32-unknown-unknown

# cargo-binstall — fetches prebuilt binaries instead of compiling from source
RUN curl -fsSL https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

RUN cargo binstall -y cargo-leptos \
    && cargo binstall -y --version 0.2.117 wasm-bindgen-cli

# Tailwind v4 standalone
RUN curl -fsSL https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-x64 \
        -o /usr/local/bin/tailwindcss \
    && chmod +x /usr/local/bin/tailwindcss

WORKDIR /app
COPY . .

ENV SQLX_OFFLINE=true
RUN cargo leptos build --release -P --project web --split

############################
# Runtime
############################
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/web /app/web
COPY --from=builder /app/target/site /app/site

ENV LEPTOS_OUTPUT_NAME=web \
    LEPTOS_SITE_ROOT=/app/site \
    LEPTOS_SITE_ADDR=0.0.0.0:8080 \
    SQLX_OFFLINE=true \
    APP_ENV=production

EXPOSE 8080
CMD ["/app/web"]
