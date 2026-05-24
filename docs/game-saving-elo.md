# Game Saving + ELO Implementation

Schema already exists (`games`, `ratings`, `rating_history` tables). `Game.id` is the UUID primary key.

## Step 0 — Fix compile errors (blocking)

- `play_board.rs` L137: add arms for `DrawOffer { .. }` and `DrawDecline` in server message match
- `play_board.rs` L204: add arm for `Resignation` in `GameOverReason` match

## Step 1 — Track move history in GameRoom

Add `pub move_history: Vec<String>` to `GameRoom::new()`.
Append each UCI string in `handle_move_made()` before the broadcast.

## Step 2 — Game creation → DB INSERT

In `state.create_game()`, after building the in-memory room, INSERT into `games`:
- `id` = `game.id`
- `status` = `'active'`
- `white_user_id`, `black_user_id`
- `mode`, `time_initial_seconds`, `time_increment_seconds`, `rated`

## Step 3 — ELO calculation

New file `crates/web/src/elo.rs`:

```rust
fn k_factor(games: i32, rating: i32) -> f64 {
    if games < 30 { 32.0 } else if rating < 2300 { 24.0 } else { 16.0 }
}

pub fn new_rating(mine: i32, theirs: i32, score: f64, games_played: i32) -> i32 {
    let expected = 1.0 / (1.0 + 10f64.powf((theirs - mine) as f64 / 400.0));
    mine + (k_factor(games_played, mine) * (score - expected)).round() as i32
}
// score: 1.0 = win, 0.5 = draw, 0.0 = loss
```

## Step 4 — DB writes on game end

`end_game()` stays sync. After each call site in `websocket.rs` and `handle_timeout`, spawn a tokio task with the DB pool from `AppState`.

Task does in order:
1. Fetch both players' rating + games count from `ratings` WHERE `mode = category`
2. Compute new ratings via ELO fn (skip if game is unrated)
3. `UPDATE games SET status='finished', result, termination, ended_at=now(), moves, final_fen, white_rating_before, black_rating_before, white_rating_after, black_rating_after WHERE id=...`
4. `UPDATE ratings SET rating=..., games=games+1, updated_at=now()` × 2
5. `INSERT INTO rating_history (user_id, mode, rating, game_id)` × 2

`result` values: `'white'` / `'black'` / `'draw'`
`termination` values: `'checkmate'` / `'resignation'` / `'timeout'` / `'draw_agreement'`

## Step 5 — Verify AppState has PgPool

If not present, add `pool: PgPool` to `AppState` and thread it through to the spawned tasks in step 4.

## Step 6 — Verify mode string matches DB CHECK

`game.config.time_control.category()` must return one of: `bullet`, `blitz`, `rapid`, `classical`, `960`.
Cross-check against the CHECK constraint in `0006_ratings.sql`.
