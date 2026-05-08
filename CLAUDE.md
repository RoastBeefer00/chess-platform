# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Dev Environment

Uses `devenv` (nix). Enter with `devenv shell` or direnv auto-activates via `.envrc`.

Postgres and Redis are managed by devenv services — start with `devenv up`.

## Commands

```bash
dev          # cargo leptos watch (hot-reload server + WASM)
migrate      # sqlx migrate run
check-all    # cargo check for both server and WASM targets

cargo leptos build --release   # production build
cargo test -p shared           # run shared crate tests
cargo check -p web --target wasm32-unknown-unknown --features hydrate
```

## Architecture

Cargo workspace with two crates:

- **`crates/shared`** — no-std-friendly types shared between server and WASM client: `Game` (wraps `shakmaty::Chess`), `Player` enum, `ServerMessage`/`ClientMessage` WebSocket message enums. Must compile to both native and WASM.
- **`crates/web`** — Leptos 0.8 full-stack app with two compile targets:
  - `--features ssr` → Axum server binary (SSR + WebSocket handler)
  - `--features hydrate` → WASM bundle (client-side hydration)

`cargo-leptos` orchestrates building both targets together. It reads config from `[package.metadata.leptos]` in `crates/web/Cargo.toml`.

## Key Constraints

- Code in `crates/shared` must not use server-only deps (tokio, axum, sqlx). Gate those behind `cfg(feature = "ssr")` in `crates/web`.
- WASM target is `wasm32-unknown-unknown`. Anything in the hydrate feature path must be WASM-compatible.
- WebSocket messages cross the server/client boundary — define them in `shared::messages` with `serde` derives.
- DB: Postgres via sqlx. Migrations go in `migrations/` (root level). `DATABASE_URL=postgresql://localhost/chess_dev`.
