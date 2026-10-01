# Known Issues

Track of intentional gaps that are not blockers for launch but should be
addressed.

Keep this file honest — an entry describing work that has since shipped is
worse than no entry, because the next person re-solves it. The
"Game state is in-memory only" entry that used to head this list was exactly
that: games now persist their moves and clocks incrementally on every move,
and a surviving instance adopts an orphaned game and replays it from the DB
rather than aborting it (see `GameStore::persist_progress`,
`AppState::adopt_game`, `AppState::reconcile_stale_active_games`).

## Chess960 games cannot be adopted

Game adoption (`AppState::adopt_game`) rebuilds a `GameRoom` by replaying the
persisted `moves` onto `Chess::default()`. A variant whose starting position
isn't the standard one has no way to reconstruct it, because the real
starting FEN is never persisted — `GameRoom::from_persisted` fails the
replay, and the game is treated as unadoptable and aborted.

Nothing ships Chess960 today (`Variant::Standard` is hardcoded at every
`GameConfig` construction site), so this is a latent constraint on adding it
rather than a live bug.

**Planned fix**: persist `initial_fen` on the `games` row — the column
already exists, unused — and replay from it instead of `Chess::default()`.

## Watch grid clocks lag on remote games

Cross-instance clocks are mirrored into the `active_games:{id}` Redis hash on
every move, so a watch tile for a game owned by another instance ticks
correctly. Between moves, though, the mirrored snapshot only refreshes on the
owning room's 10s heartbeat, so a remote tile can be up to ~10s stale right
after a reconnect. Locally-owned tiles read the live `GameRoom` and have no
such gap.

**Impact**: cosmetic, and only visible with more than one instance running.

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
