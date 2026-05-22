# Chess Clocks — Implementation Guide

## Goal

Server-authoritative chess clocks with:
- Real-time countdown displayed on both clients
- Increment applied per move (Fischer style for v1)
- Timeout detection while a player sits idle on their turn
- Survives client reconnects (server pushes current clock state on connect)

Server is source of truth. Clients only display.

---

## Data Model

### Game (in `crates/shared/src/game.rs`)

Replace the current `Instant` fields with:

```rust
pub struct Game {
    pub id: Uuid,
    pub position: Chess,
    pub config: GameConfig,            // already exists
    pub white_player: Uuid,
    pub black_player: Uuid,

    // Clock state — all on the server.
    pub white_ms_left: i64,
    pub black_ms_left: i64,
    pub last_move_at: Option<Instant>, // None until first move
}
```

Initial values for `white_ms_left` / `black_ms_left` come from `config.time_control.initial_time`.

`Instant` is server-only — gate it behind `#[cfg(feature = "ssr")]` in the shared crate, or move clock state out of `Game` into a server-only struct alongside `GameRoom`. Recommended: put `last_move_at` on `GameRoom` (server-only) and only the millis on `Game` (so the client can read them too).

### GameRoom (in `crates/web/src/game_room.rs`)

Add:

```rust
pub struct GameRoom {
    // ... existing fields ...
    pub last_move_at: Option<Instant>,
    pub timeout_task: Option<tokio::task::JoinHandle<()>>,
}
```

`timeout_task` holds the handle to the per-move timer so we can cancel it when the next move arrives.

---

## Server Logic

### On every `GameClientMessage::MoveMade`

```text
1. elapsed = match last_move_at {
       Some(t) => now - t,
       None => Duration::ZERO,            // first move of game
   }
2. mover_ms = if mover == White { white_ms_left } else { black_ms_left }
3. mover_ms -= elapsed.as_millis()
4. if mover_ms <= 0:
       broadcast GameOver { winner: opposite(mover), reason: Timeout }
       mark game finished, persist if needed
       return
5. mover_ms += increment_ms()              // Fischer
6. write mover_ms back into game state
7. cancel timeout_task if Some
8. last_move_at = Some(now())
9. play the move on the position
10. broadcast MoveMade with new clock snapshot
11. schedule new timeout_task for opponent (now to move):
        let remaining = opponent_ms;
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(remaining as u64)).await;
            handle_timeout(game_id, opponent_color).await;
        });
        store handle in timeout_task
```

### `handle_timeout(game_id, color)`

When the scheduled sleep fires, the player MIGHT have already moved. Validate before acting:

```text
1. lock the GameRoom
2. is it still `color`'s turn? (read from position)
3. is the game still ongoing? (status check)
4. is enough time actually elapsed? (now - last_move_at >= remaining)
5. if all yes:
       set clock for `color` to 0
       broadcast GameOver { winner: opposite(color), reason: Timeout }
       mark finished
       persist if needed
6. if any check fails: bail (the next move handler already rescheduled)
```

### When clocks actually start

Pick one rule:
- **Both players connected** — start timing on second player's join.
- **First move** — clock starts running for opponent only after white plays move 1.

Recommend the second (lichess-style). It's simpler: `last_move_at` stays `None` until the first move handler runs. Until then, no clock is running.

---

## Wire Protocol

Extend `GameServerMessage` in `crates/shared/src/messages/game.rs`:

```rust
pub enum GameServerMessage {
    UserJoined { /* existing */ },
    UserLeft { /* existing */ },
    MoveMade {
        uci: String,
        white_ms_left: i64,
        black_ms_left: i64,
        turn: Side,
        sent_at_ms: i64,         // server wall clock at broadcast
    },
    ClockSync {
        white_ms_left: i64,
        black_ms_left: i64,
        turn: Side,
        sent_at_ms: i64,
    },
    GameOver {
        winner: Option<Side>,    // None for draw
        reason: GameOverReason,
    },
    Chat { /* existing */ },
}

pub enum GameOverReason {
    Checkmate,
    Stalemate,
    Timeout,
    Resignation,
    DrawAgreement,
    Insufficient,
    FiftyMove,
    Repetition,
    Abandonment,
}
```

`sent_at_ms` lets clients estimate when the snapshot was actually fresh (vs when they received it), so countdown calculations stay roughly accurate under network lag.

`ClockSync` is broadcast once when a client joins/reconnects mid-game so they have a starting point.

---

## Client

### `Clock` component (new file `crates/web/src/components/clock.rs`)

Props:

```rust
#[component]
pub fn Clock(
    /// Total ms left at last server snapshot.
    snapshot_ms: Signal<i64>,
    /// `sent_at_ms` from the server snapshot (server wall clock).
    snapshot_sent_at_ms: Signal<i64>,
    /// Whether this side is currently on the move (clock should tick down).
    is_active: Signal<bool>,
) -> impl IntoView
```

Internal state:
- `displayed_ms: RwSignal<i64>` — what to render right now.

Tick loop (hydrate-only):
```text
on mount:
    set_interval(100ms):
        if is_active.get():
            // Time elapsed locally since the snapshot was received.
            let local_now_ms = current_local_ms();
            let elapsed = local_now_ms - snapshot_received_at;
            displayed_ms.set(snapshot_ms.get() - elapsed)
        else:
            displayed_ms.set(snapshot_ms.get())
```

When a new snapshot arrives (via signal update), reset `snapshot_received_at` to local now.

### Display formatting

- `MM:SS` while > 20 s
- `MM:SS.t` (with tenths) when ≤ 20 s — adds urgency
- Bold red text when ≤ 10 s
- Gray/dim when not the active side

Use `format!("{:02}:{:02}", mins, secs)` for the MM:SS part.

### `PlayBoard` wiring

`PlayBoard` already owns the WebSocket and pushes server messages into signals. Add three new signals:

```rust
let white_ms = RwSignal::new(initial_time_ms);
let black_ms = RwSignal::new(initial_time_ms);
let sent_at_ms = RwSignal::new(0_i64);
```

On `MoveMade` / `ClockSync` messages, update all three.

In the view:

```rust
<Clock
    snapshot_ms=white_ms.into()
    snapshot_sent_at_ms=sent_at_ms.into()
    is_active=Signal::derive(move || position.get().turn() == Color::White)
/>
```

Place each Clock in the BoardUser row slot you reserved earlier (`<div class="flex flex-row items-center"> ... </div>` next to `<BoardUser ... />`).

---

## Phased Build Order

Build and test in this order — each phase is independently functional.

### Phase 1 — Data model

- Update `Game` struct: add `white_ms_left`, `black_ms_left`.
- Update `GameRoom`: add `last_move_at: Option<Instant>`, `timeout_task: Option<JoinHandle<()>>`.
- Initial clock values come from `config.time_control.initial_time` in `GameRoom::new`.
- Compile errors guide you to every call site that needs updating.

### Phase 2 — Server move handler updates clocks

- In `websocket.rs` `MoveMade` arm, compute elapsed, deduct, add increment, store back.
- Broadcast `MoveMade` with new clock snapshot fields.
- No timeout task yet — players can run out of time without server noticing.

### Phase 3 — Static Clock component

- Create `components/clock.rs`.
- Renders `snapshot_ms` formatted as MM:SS. No ticking yet.
- Plug into `PlayBoard` view.
- Watch the value update on every move. Confirm wire-up works.

### Phase 4 — Local ticking

- `set_interval(100ms)` in Clock — only when `is_active`.
- Decrement display based on local elapsed since snapshot received.
- Confirm clock visually counts down between moves.

### Phase 5 — Timeout task on server

- Spawn `tokio::task` after each move that sleeps for opponent's remaining time.
- On wake, validate state, broadcast `GameOver { reason: Timeout }`.
- Cancel previous task before scheduling new one.
- Test: open game, let one side's clock run out — game should end automatically.

### Phase 6 — `ClockSync` on connect

- On WebSocket connect (or on reconnect), server pushes a `ClockSync` message before the loop.
- Reconnecting client gets immediate state.

### Phase 7 — Polish

- Format tenths under 20 s.
- Red text under 10 s.
- Dim inactive side.
- Persist clock state to Postgres after each move (optional — survives restarts).

---

## Edge Cases & Gotchas

### Self-reference in timeout task

The spawned task needs to call back into `GameRoom`. Approaches:
- **Easiest**: pass an `Arc<Mutex<GameRoom>>` clone into the task closure.
- **Cleaner**: store a `Weak<Mutex<GameRoom>>` on the room itself, `weak.upgrade()` inside the task.

`AppState::create_game` should hand the new room a reference back to itself.

### Race: move and timeout fire simultaneously

Both grab the `Mutex<GameRoom>`. Whoever wins acts. The other sees state changed and bails. `handle_timeout` MUST check "is it still that side's turn / has time actually elapsed" before declaring timeout.

### First-move latency

`last_move_at` is `None` until the first move. The first move handler treats elapsed as zero. No special timer scheduling needed before that (white's clock isn't running yet).

### Server restart mid-game

In-memory state lost. Two options:
- Accept loss: games started before restart are abandoned. Acceptable early on.
- Persist clocks to Postgres after each move (`UPDATE games SET white_ms = $1, black_ms = $2, last_move_at = $3 WHERE id = $4`). On restart, rebuild `GameRoom` from row.

Defer until you actually deploy more than once a day with active games.

### Disconnect

Recommended: clocks keep running for disconnected players (lichess behavior). They lose on time if they don't reconnect.

If you want pause-on-disconnect: complicated. Don't bother for v1.

### Lag compensation (clock going UP)

Not in this design. Clock only ticks down. To add it later: server tracks per-player RTT, credits ms back on each move (capped to prevent abuse). Layer on top of this design — doesn't require refactor.

### Multiple machines

This design assumes one fly machine (one process). Timer tasks are local to that machine. Scaling horizontally requires either:
- Sticky routing (both players + spectators land on same machine)
- Distributed clock state (Redis-backed) — much harder

Defer until you scale.

---

## Test Plan

- Two-player game, normal moves — clocks tick down correctly, increment applied.
- One side runs out of time while opponent waits — server broadcasts `GameOver { Timeout }`.
- Player moves with 0.1s left — clock displays sub-second correctly.
- Reconnect mid-game — `ClockSync` lands, display resumes from correct value.
- Network lag — clock may appear slightly low but never displays negative.

---

## Future Extensions

- **Bronstein / US delay** modes — adjust the elapsed calculation in the move handler.
- **Pre-move support** — client queues moves while opponent thinks. Validate + send when turn flips. Independent of clock logic.
- **Time-control-aware abort** — if game ends in < 2 moves, no rating change (lichess does this).
- **Berserk** — option to halve your starting time for rating bonus. Tournament feature.
