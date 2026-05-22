# Rematch — Implementation Guide

## Goal

After a game ends, either player can offer a rematch. The other player accepts or declines. On accept, a new game is created with the **same time control** and **swapped colors**. All WebSocket subscribers (both players + any spectators) are notified of the new game id and can navigate to it.

## Why option 1 (offer/accept) over alternatives

| Approach | Pros | Cons |
|----------|------|------|
| **Offer / accept via WS** (chosen) | Standard chess UX. Mutual consent. Spectators auto-follow via broadcast. Fast (no queue wait). | New message types + offer state on game room. |
| Special matchmaking bucket per pair | Reuses queue infra. | No consent enforcement. If only one clicks, they get matched with someone else. Wrong UX. |
| Direct room creation | Trivial implementation. | Forces second player into game without consent. Bad UX. |

The chosen approach matches lichess + chess.com. Spectators get redirected via the same broadcast, so no special handling.

---

## Protocol

### New `GameClientMessage` variants

```rust
RematchOffer,
RematchAccept,
RematchDecline,
RematchCancel,   // offerer withdraws before opponent answers
```

### New `GameServerMessage` variants

```rust
RematchOffered { from: Uuid },        // someone offered; show Accept/Decline UI to the other side
RematchAccepted { new_game_id: Uuid }, // both sides redirect to this game
RematchDeclined,                       // offer was rejected; offerer sees "they said no"
RematchCanceled,                       // offerer withdrew before answer
```

All `RematchOffered` / `RematchAccepted` / etc. are broadcast to the **entire room** (players + spectators). Spectators ignore the Accept/Decline UI but still get `RematchAccepted` for redirect.

---

## Server State

Add to `GameRoom`:

```rust
pub rematch_offer: Option<Uuid>,   // user_id of the offerer, None if no pending offer
```

Cleared when:
- The offer is accepted (game starts)
- The offer is declined
- The offer is canceled
- The offerer disconnects (their WS task removes player + clears offer)
- A new game is created (defensive — should already be cleared)

---

## Server Handling (in `game_websocket`)

### `RematchOffer`

1. Game must be in `Finished` status (no rematch mid-game).
2. Sender must be one of the two players (not a spectator).
3. If `rematch_offer` is already set to the same user, no-op.
4. If `rematch_offer` is set to the OTHER user (implicit acceptance race) — treat as if Accept was sent.
5. Otherwise: set `rematch_offer = Some(sender_id)`, broadcast `RematchOffered { from: sender_id }`.

### `RematchAccept`

1. Must be the other player (not the offerer, not a spectator).
2. `rematch_offer` must be `Some(other_id)`.
3. Create new game:
   - **Swap colors**: `new_white = old_black`, `new_black = old_white`.
   - Same `GameConfig` (time control, variant, rated).
   - Call `state.create_game(new_white, new_black, config)` → returns `new_game_id`.
4. Broadcast `RematchAccepted { new_game_id }` to old room.
5. Clear `rematch_offer = None`.
6. Old game's room can stay around briefly for the broadcast; clients will all navigate away.

### `RematchDecline`

1. Must be the other player.
2. Clear `rematch_offer = None`.
3. Broadcast `RematchDeclined`.

### `RematchCancel`

1. Must be the original offerer.
2. Clear `rematch_offer = None`.
3. Broadcast `RematchCanceled`.

### Offerer disconnect

In the WS cleanup path (when `input.next().await` returns `None`):
- If `rematch_offer == Some(disconnecting_user_id)`, clear it.
- Broadcast `RematchCanceled` so the opponent's UI clears the pending offer.

---

## Client UI (GameOverModal)

State the modal needs to render:

```rust
enum RematchState {
    Idle,                  // initial — Rematch button shown
    Offering,              // we sent an offer, waiting for opponent — "Cancel" button
    OfferedToUs { from: Uuid },  // opponent offered — show "Accept" + "Decline"
    Declined,              // opponent declined our offer — show message briefly
}
```

Track this state in PlayBoard (or modal-local) and pass it into GameOverModal. Buttons:

- **Idle**: `[Rematch]` `[New Game]` `[X]`
- **Offering**: `Waiting for opponent...` `[Cancel]` `[New Game]` `[X]`
- **OfferedToUs**: `Opponent wants a rematch` `[Accept]` `[Decline]` `[X]`
- **Declined**: `They declined` (auto-revert to Idle after 2s) `[New Game]` `[X]`

### WS message handling (PlayBoard)

```rust
GameServerMessage::RematchOffered { from } => {
    if from != my_user_id {
        rematch_state.set(RematchState::OfferedToUs { from });
    }
    // if from == my_user_id, it's the echo of our own offer — no state change
}
GameServerMessage::RematchAccepted { new_game_id } => {
    navigate(&format!("/play/{new_game_id}"), NavigateOptions::default());
}
GameServerMessage::RematchDeclined => {
    rematch_state.set(RematchState::Declined);
    // optional: timer to revert to Idle after a few seconds
}
GameServerMessage::RematchCanceled => {
    rematch_state.set(RematchState::Idle);
}
```

### Spectator handling

Spectators receive the same broadcasts. UI distinguishes:
- They never see Rematch/Accept/Decline buttons (modal could be hidden entirely for spectators, or just show the outcome).
- On `RematchAccepted`, they navigate too — automatically follow the players into the new game.

---

## Color Swap

On accept:

```rust
let new_white = old_room.game.black_player;
let new_black = old_room.game.white_player;
let new_id = state.create_game(new_white, new_black, old_room.game.config.clone()).await;
```

Ensures the player who was black gets a turn as white in the next game. Lichess does this.

---

## Implementation Phases

Each phase compiles and is releasable on its own.

### Phase 1 — Protocol (5 min)
- Add the new `GameClientMessage` and `GameServerMessage` variants in `crates/shared/src/messages/game.rs`.
- Update both client and server match arms to handle (or ignore for now) the new variants.

### Phase 2 — Room state (5 min)
- Add `rematch_offer: Option<Uuid>` to `GameRoom` (`crates/web/src/game_room.rs`).
- Initialize in `GameRoom::new`.

### Phase 3 — Server handling (30 min)
- In `game_websocket`'s message loop, add match arms for the four rematch client messages.
- Implement color swap in the `RematchAccept` arm.
- Clear `rematch_offer` on disconnect.
- Broadcast appropriate server messages from each arm.

### Phase 4 — Client UI (20 min)
- Add `RematchState` signal in `PlayBoard`.
- Update `GameOverModal` to take rematch state + four callbacks (Offer / Accept / Decline / Cancel).
- Conditional button rendering based on state.

### Phase 5 — Navigate on accept (5 min)
- In `PlayBoard`'s WS handler, navigate on `RematchAccepted`.
- Verify spectators also navigate.

---

## Edge Cases

- **Both click Rematch at the same time.** Server gets two `RematchOffer` messages from different players. Second handler sees `rematch_offer == Some(other)` and treats it as an Accept. Game created, both navigate.
- **Offerer disconnects.** Cleanup broadcasts `RematchCanceled`. Opponent's UI reverts to Idle.
- **Spectator clicks Rematch.** Server rejects (must be a player). UI doesn't show the button to spectators in the first place.
- **Rematch offered before game ended (race).** Server checks `status != GameStatus::Finished`, rejects.
- **Multiple games chained.** Each new game spawns its own `GameRoom` from `state.create_game`. The old `GameRoom` lingers until `state.games` map is pruned (separate concern — eventual cleanup).

---

## Future Polish

- **Offer timeout** — auto-decline after N seconds if no response. Lichess doesn't do this; offers persist until manually cleared.
- **Stats** — track rematch acceptance rate per user.
- **Tournament integration** — disallow rematches in tournament games (force pairing system).
- **Score keeping** — track win/loss/draw across consecutive rematches between same two players, display in the modal.
