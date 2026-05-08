# shared

Types shared between the server (`web --features ssr`) and WASM client (`web --features hydrate`).

Must compile to both native and `wasm32-unknown-unknown` — no server-only dependencies allowed.

## Contents

| Module | Description |
|---|---|
| `game` | `Game` struct wrapping `shakmaty::Chess` |
| `player` | `Player` enum and role types |
| `messages` | `ServerMessage` / `ClientMessage` WebSocket message enums |

## Rules

- No `tokio`, `axum`, or `sqlx` — these are SSR-only deps
- All public types need `serde` derives (messages cross the WebSocket boundary)
- Test with `cargo test -p shared`
