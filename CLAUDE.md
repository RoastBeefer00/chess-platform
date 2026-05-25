# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Dev Environment

Uses `devenv` (nix). Enter with `devenv shell` or direnv auto-activates via `.envrc`.

Postgres and Redis are managed by devenv services — start with `devenv up`.

Required env vars (copy `.env.example` to `.env` and fill in):
- `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET`, `GITHUB_REDIRECT_URI`
- `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GOOGLE_REDIRECT_URI`
- `DATABASE_URL=postgresql://localhost/chess_dev`
- `REDIS_URL=redis://127.0.0.1:6379`
- `ENV=development` (or `production`)

## Commands

```bash
# Dev
devenv up        # start Postgres + Redis services
dev              # cargo leptos watch (hot-reload server + WASM); defined in devenv.nix
migrate          # sqlx migrate run

# Checks
check-all        # cargo check for both server and WASM targets
clippy           # cargo clippy for both targets

# Production
cargo leptos build --release

# Tests
cargo test -p shared                                                         # shared crate unit tests
npx playwright test                                                          # e2e tests (requires running server)

# Manual WASM check
cargo check -p web --target wasm32-unknown-unknown --features hydrate

# Kill dev server
kill-dev
```

## Architecture

Cargo workspace at repo root. Two crates:

- **`crates/shared`** — WASM-safe shared types: `Game`, `GameInfo`, `GameStatus`, `PlayerInfo`, `Side`, `PlayerRole`, `GameConfig`, `TimeControl`, `Variant`, `Category`, and all WebSocket message enums. Must compile to both native and `wasm32-unknown-unknown`. No server-only deps allowed.
- **`crates/web`** — Leptos 0.8 full-stack app:
  - `--features ssr` → Axum server binary (SSR + API + WebSocket handler)
  - `--features hydrate` → WASM bundle (client-side hydration + interactivity)

`cargo-leptos` orchestrates both builds. Configuration lives in `[package.metadata.leptos]` in `crates/web/Cargo.toml`.

### Server Modules (`crates/web/src/`, SSR only)

| Module | Role |
|--------|------|
| `main.rs` | Axum server entry: DB/Redis init, session middleware, OAuth routes, rate limiting, CSP headers |
| `app.rs` | Leptos router and shell HTML component |
| `state.rs` | `AppState` (in-memory game rooms, auth backend, DB stores, Redis client); Lua-based Redis matchmaking script |
| `websocket.rs` | WebSocket server_fn: moves, draw/rematch offers, resignations, chat, timeout |
| `game_room.rs` | `GameRoom` (in-memory game state + clocks + broadcast channel), `handle_move_made`, `handle_timeout` |
| `game.rs` | `get_game_info` server function (fetch game + player info from DB) |
| `elo.rs` | `k_factor()`, `new_rating()` — K varies by games played and rating level |
| `auth/` | GitHub OAuth2 and Google OpenID Connect flows, `AuthBackend`, `User` struct |
| `db/` | `user_store`, `game_store` (atomic finalization with ELO update), `rating_store` |
| `matchmaking/` | `search_for_game` server function; Redis-bucket pairing by rating window |

### Client Modules (`crates/web/src/`, WASM hydrate)

| Module | Role |
|--------|------|
| `components/chess_board.rs` | Board rendering, square highlights, legal move dots |
| `components/square.rs` | Individual square with drag-and-drop |
| `components/play_board.rs` | Main game UI: WebSocket connection, move dispatch, clock sync |
| `components/clock.rs` | Countdown display |
| `components/game_over_modal.rs` | End-of-game result + rematch/draw UI |
| `components/matchmaking_modal.rs` | Queue waiting UI |
| `components/nav.rs` | Navigation bar |
| `components/auth.rs` | `provide_current_user()` context provider |
| `pages/` | Route page components: home, login, play, create_username, not_found |
| `sound.rs` | Audio playback (move, capture, check, game start, etc.) — conditionally compiled for WASM |

### Real-Time Flow

1. Client calls `search_for_game` server function → adds to Redis rating bucket, Lua script pairs two users
2. Both users are redirected to `/game/:game_id`
3. Client opens WebSocket via `game_websocket` server_fn
4. Server maintains `GameRoom` in `AppState.games` (tokio broadcast channel for multi-client sync)
5. Moves validated with `shakmaty`, clocks updated in `GameRoom`, outcome checked after each move
6. On game end: tokio::spawn runs atomic DB transaction (update game status + ELO + rating history)

## Key Constraints

- `crates/shared` must not use server-only deps (tokio, axum, sqlx). Any server-only code in `crates/web` must be gated behind `#[cfg(feature = "ssr")]`.
- WASM target is `wasm32-unknown-unknown`. All code in the `hydrate` feature path must be WASM-compatible. Avoid threading primitives, native file I/O, or OS syscalls.
- WebSocket messages cross the server/client boundary — always define them in `shared::messages` with `#[derive(Serialize, Deserialize)]`.
- SQLx uses compile-time checked queries. Run `cargo sqlx prepare` if adding new queries and you need offline mode; `DATABASE_URL` must be set for `cargo check` to pass with live query checking.
- Migrations live in `migrations/` (workspace root, not inside any crate).

## Database Schema

Postgres (`chess_dev`). All tables use UUID primary keys except `rating_history` (BIGSERIAL).

| Table | Key Columns |
|-------|-------------|
| `users` | id (UUID), email (unique), username (unique), avatar_url, bio, country (CHAR(2)), created_at |
| `oauth_accounts` | user_id (fk), provider, provider_user_id (unique per provider) |
| `sessions` | id, data (bytea), expiry_date |
| `games` | id, mode (bullet/blitz/rapid/classical/960), time_initial/increment/delay, rated, white/black_user_id, status (waiting/active/finished/aborted), result, termination, FEN strings, ratings before/after, timestamps |
| `matchmaking_queue` | user_id (pk), mode, time controls, rated, rating, joined_at |
| `ratings` | (user_id, mode) pk, rating (default 1500), games, updated_at |
| `rating_history` | id (BIGSERIAL), user_id, mode, rating, game_id (fk), recorded_at |

A trigger on `users` auto-seeds all 5 game modes to 1500 on new user insert (migration 0006).

## Game Modes & Types

- **Categories**: `Bullet` (≤3 min), `Blitz` (3–10 min), `Rapid` (10–30 min), `Classical` (>30 min)
- **Variants**: `Standard`, `Chess960`
- **Time modes**: `Increment` (add seconds after move), `Delay` (bronstein delay)
- **Rating modes**: `Casual` (unrated), `Rated`

## ELO Implementation

Located in `crates/web/src/elo.rs`:
- Expected win probability: `1 / (1 + 10^((opponent - self) / 400))`
- K-factor: 32 for <30 games played, 24 for rating <2300, 16 for elite
- Rating updates are part of the atomic game finalization transaction in `db/game_store.rs`

## Security Notes

- **CSP**: Allows `unsafe-inline` for scripts (required by Leptos hydration). Rely on Leptos's default HTML escaping for XSS prevention.
- **Rate limiting**: `tower_governor` with `SmartIpKeyExtractor` — trusts `X-Forwarded-For`. Ensure deployment is behind a trusted reverse proxy (Fly.io handles this).
  - OAuth routes: 6 req/s, API routes: 2 req/s
- **CSRF**: OAuth flows use state parameter (GitHub) and nonce+state (Google OIDC).
- **Sessions**: `tower-sessions` backed by Redis, HTTP-only cookies.

## Deployment

Hosted on Fly.io (`fly.toml`, app: `chess-rs`, region: `dfw`).

```bash
fly deploy   # uses Dockerfile (multi-stage Rust builder → Debian slim runtime)
```

Docker build installs `cargo-leptos`, `tailwindcss`, and `wasm32-unknown-unknown` target. Runtime image copies the server binary and `target/site/` (static assets + WASM pkg).

Production env: `SQLX_OFFLINE=true`, `LEPTOS_SITE_ADDR=0.0.0.0:8080`.

## Known Issues

Tracked in `KNOWN_ISSUES.md`:
1. **In-memory game state** lost on server restart — plan to persist every move to DB
2. **WebSocket frame size** unbounded at WS layer — plan to use hand-rolled axum WS route with `max_message_size`
3. **CSP `unsafe-inline`** — mitigated by Leptos escaping; tracked for future nonce-based CSP
4. **Rate limiting trusts `X-Forwarded-For`** — safe only behind trusted proxy; confirm before scaling

## Conventions

- New server-only code goes in `crates/web/src/` behind `#[cfg(feature = "ssr")]` or in a module imported only from SSR feature paths.
- New shared types go in `crates/shared/src/`, must be `no_std`-friendly and serde-derivable.
- UI components go in `crates/web/src/components/`, page routes in `crates/web/src/pages/`.
- WebSocket message variants: add to `shared::messages::game` (game WS) or `shared::messages::matchmaking`; handle in `websocket.rs` on server and `play_board.rs` on client.
- SQL migrations: add a new numbered file in `migrations/`, run `migrate` to apply.
- Do not add `tokio`, `axum`, `sqlx`, or any native-only crate to `crates/shared/Cargo.toml`.
