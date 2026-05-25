# Known Issues

Track of intentional gaps that are not blockers for launch but should be addressed.

## Game state is in-memory only

`GameRoom`s live inside `AppState.games` (`HashMap<Uuid, Arc<Mutex<GameRoom>>>`).
A server restart drops every in-progress game — clients reconnect to a `game not
found` error and lose their position, clocks, and rating-affecting result.

**Impact**: every deploy aborts active games. Disruptive but not a data loss
risk against the DB.

**Planned fix**: persist game state on every move, replay from DB on cold start.
Tracked in `docs/game-saving-elo.md`.

## WebSocket frame size is bounded only at the HTTP body layer

The global `RequestBodyLimitLayer::new(1024 * 1024)` (in `main.rs`) caps each
HTTP request body to 1 MiB. `server_fn`'s Websocket protocol does not expose a
per-frame `max_message_size`, so very large WebSocket frames are technically
possible after the upgrade. In practice the only client messages are short
UCI strings, chat (not yet implemented), and small rematch/draw markers.

**Planned fix**: when chat ships, swap to a hand-rolled `axum::extract::ws`
route with explicit `max_message_size`.

## CSP allows `'unsafe-inline'` and `'unsafe-eval'` for scripts

Leptos's hydration emits inline `<script>` blocks containing the initial app
state, so `'unsafe-inline'` is required. wasm-bindgen's generated JS glue
additionally uses `new Function(...)` in some code paths (notably triggered
when dynamic views like modals mount), so `'unsafe-eval'` is also required.

We rely on Leptos's default HTML escaping (`view!` macro) to prevent XSS
since CSP no longer blocks injected `<script>` tags or string-evaluated JS.

**Action**: never render user-controlled strings as raw HTML
(`inner_html=`, `<div inner_html=...>`, manually-constructed `view!`
strings). If you must, sanitize first.

A stricter alternative — hash-pinning each emitted script — breaks every
time Leptos's hydration template changes.

## Rate limiting trusts `X-Forwarded-For`

`SmartIpKeyExtractor` reads `X-Forwarded-For` when present and falls back to
the peer socket address otherwise. Deployed **behind a trusted proxy**
(Cloudflare, nginx, Caddy) this gives accurate per-client IPs. Deployed
**directly to the internet** a malicious client can spoof the header and
bypass per-IP limits.

**Action before public launch**: confirm deployment topology. If direct,
swap to `PeerIpKeyExtractor` in `main.rs`.
