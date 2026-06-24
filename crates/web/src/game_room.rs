use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use shakmaty::{
    fen::Fen,
    uci::UciMove,
    zobrist::Zobrist64,
    Chess, Color, EnPassantMode, KnownOutcome, Move, Outcome, Position as _,
};
use tokio::{
    sync::{
        broadcast::{self, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};
use tracing::instrument;

use shared::{
    messages::GameOverReason, Game, GameServerMessage, GameStatus, PlayerRole, Side, TimeMode,
};
use uuid::Uuid;

use crate::db::{GameFinalization, GameStore};

const BROADCAST_CAPACITY: usize = 32;

/// Jitter margin added to a mover's measured RTT when bounding how much
/// network latency we forgive on a move. Wider = more lenient toward laggy
/// clients (and easier to abuse by faking think time); narrower = stricter.
const RTT_CAP_SLACK_MS: i64 = 200;

#[derive(Debug, thiserror::Error)]
pub enum MoveError {
    #[error("invalid uci: {0}")]
    InvalidUci(String),
    #[error("illegal move: {0}")]
    Illegal(String),
    #[error("not your turn")]
    NotYourTurn,
    #[error("flag fall")]
    FlagFall,
}

pub struct TimeoutPlan {
    pub next_color: Color,
    pub ms_until_flag: i64,
}

pub enum MoveOutcome {
    /// Move applied normally; schedule a timeout for the next player.
    Continuing(TimeoutPlan),
    /// Move ended the game (checkmate, stalemate, insufficient material).
    /// Carries the finalization plan when this was the transition to Finished;
    /// `None` if the game was already finished (defensive — shouldn't normally
    /// happen on this code path).
    Ended(Option<GameFinalization>),
}

#[derive(Debug)]
pub struct GameRoom {
    pub game: Game,
    pub status: GameStatus,
    /// Refcounted set of connected users. Multiple WebSocket sessions per user
    /// (e.g. laptop + phone for the same account) each `add_player` on connect
    /// and `remove_player` on disconnect; the user only counts as "left" when
    /// the count reaches zero.
    pub connected: HashMap<Uuid, u32>,
    /// Server-estimated RTT (ms) for each connected player. Updated on every
    /// Ping bucket change; cleared when the user fully disconnects.
    rtt_ms: HashMap<Uuid, u32>,
    tx: Sender<GameServerMessage>,
    pub last_move_at: Option<Instant>,
    pub timeout_task: Option<JoinHandle<()>>,
    pub abort_task: Option<JoinHandle<()>>,
    pub abort_side: Option<Side>,
    pub abort_deadline_ms: Option<i64>,
    pub rematch_offer: Option<Uuid>,
    pub draw_offer: Option<Uuid>,
    pub move_history: Vec<String>,
    /// Zobrist hash → occurrence count for the current game, used to detect
    /// threefold repetition. Seeded with the starting position at construction.
    position_counts: HashMap<Zobrist64, u8>,
    /// Set when `end_game` runs; allows the websocket join handler to replay
    /// the `GameOver` event to a client that reconnects after the game
    /// finished (otherwise they'd see a frozen board with no modal).
    pub end_reason: Option<GameOverReason>,
    /// Cumulative score across all games in this rematch series (white, black). Draws give 0.5.
    pub session_score: (f32, f32),
}

impl GameRoom {
    pub fn new(game: Game, session_score: (f32, f32)) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let start_hash = game
            .position
            .zobrist_hash::<Zobrist64>(EnPassantMode::Legal);
        let mut position_counts = HashMap::new();
        position_counts.insert(start_hash, 1u8);
        GameRoom {
            game,
            status: GameStatus::WaitingForOpponent,
            connected: HashMap::new(),
            rtt_ms: HashMap::new(),
            tx,
            last_move_at: None,
            timeout_task: None,
            abort_task: None,
            abort_side: None,
            abort_deadline_ms: None,
            rematch_offer: None,
            draw_offer: None,
            move_history: Vec::new(),
            position_counts,
            end_reason: None,
            session_score,
        }
    }

    pub fn subscribe(&self) -> Receiver<GameServerMessage> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, msg: GameServerMessage) {
        let _ = self.tx.send(msg);
    }

    /// Number of distinct connected users (not number of sessions).
    pub fn player_count(&self) -> usize {
        self.connected.len()
    }

    pub fn update_rtt(&mut self, id: Uuid, rtt: u32) {
        self.rtt_ms.insert(id, rtt);
    }

    pub fn rtt_of(&self, id: Uuid) -> Option<u32> {
        self.rtt_ms.get(&id).copied()
    }

    pub fn user_side(&self, id: Uuid) -> Option<Side> {
        if id == self.game.white_player {
            Some(Side::White)
        } else if id == self.game.black_player {
            Some(Side::Black)
        } else {
            None
        }
    }

    pub fn get_position(&self) -> Chess {
        self.game.position.clone()
    }

    #[instrument(skip(self))]
    pub fn add_player(&mut self, player_id: Uuid) -> (PlayerRole, bool) {
        let was_waiting = matches!(self.status, GameStatus::WaitingForOpponent);

        let role = if player_id == self.game.white_player {
            *self.connected.entry(player_id).or_insert(0) += 1;
            PlayerRole::Player(Color::White.into())
        } else if player_id == self.game.black_player {
            *self.connected.entry(player_id).or_insert(0) += 1;
            PlayerRole::Player(Color::Black.into())
        } else {
            PlayerRole::Spectator
        };

        if self.connected.contains_key(&self.game.white_player)
            && self.connected.contains_key(&self.game.black_player)
        {
            self.status = GameStatus::Ongoing;
        }

        // True only on the exact WaitingForOpponent → Ongoing transition with no
        // moves yet (not on reconnects or when the game has already progressed).
        let started = was_waiting
            && matches!(self.status, GameStatus::Ongoing)
            && self.move_history.is_empty();

        (role, started)
    }

    /// Decrement the session count for this user; remove the entry when it
    /// hits zero. Safe to call for unknown UUIDs (spectators) — it's a no-op.
    #[instrument(skip(self))]
    pub fn remove_player(&mut self, id: Uuid) {
        if let Some(count) = self.connected.get_mut(&id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.connected.remove(&id);
                self.rtt_ms.remove(&id);
            }
        }
    }

    pub fn current_player(&self) -> Option<Uuid> {
        let id = match self.game.position.turn() {
            Color::White => self.game.white_player,
            Color::Black => self.game.black_player,
        };
        // Only consider it "their turn" if they're actually connected.
        self.connected.contains_key(&id).then_some(id)
    }

    #[instrument(skip(self))]
    pub fn make_move(&mut self, mv: Move) -> Result<(), String> {
        let chess = self.get_position();
        match chess.play(mv) {
            Ok(pos) => {
                self.game.position = pos;
                Ok(())
            }
            Err(_) => Err("Illegal move".to_string()),
        }
    }

    /// Transition the game to Finished, broadcast the final clock + outcome,
    /// cancel any pending timeout, and return a finalization snapshot that
    /// the caller should hand to `GameStore::finalize_game` (typically via
    /// `tokio::spawn`). Returns `None` if the game was already finished —
    /// nothing to broadcast or persist a second time.
    #[instrument(skip(self), fields(game_id = %self.game.id, ?outcome, ?reason))]
    pub fn end_game(
        &mut self,
        outcome: KnownOutcome,
        reason: GameOverReason,
    ) -> Option<GameFinalization> {
        if matches!(self.status, GameStatus::Finished(_)) {
            return None;
        }
        self.status = GameStatus::Finished(Outcome::Known(outcome));
        self.end_reason = Some(reason.clone());

        match outcome {
            KnownOutcome::Decisive {
                winner: Color::White,
            } => self.session_score.0 += 1.0,
            KnownOutcome::Decisive {
                winner: Color::Black,
            } => self.session_score.1 += 1.0,
            KnownOutcome::Draw => {
                self.session_score.0 += 0.5;
                self.session_score.1 += 0.5;
            }
        }

        // Push the final clock snapshot so clients display the true ending values
        // (e.g. 0.0 for the side that flagged) instead of whatever their local
        // interval extrapolated to.
        let sent_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        self.broadcast(GameServerMessage::ClockSync {
            white_ms_left: self.game.white_ms_left,
            black_ms_left: self.game.black_ms_left,
            turn: self.game.position.turn().into(),
            sent_at_ms,
            clock_running: false,
        });

        let winner = match outcome {
            KnownOutcome::Decisive { winner } => Some(Side::from(winner)),
            KnownOutcome::Draw => None,
        };
        let (white_wins, black_wins) = self.session_score;
        self.broadcast(GameServerMessage::GameOver {
            winner,
            reason: reason.clone(),
            white_wins,
            black_wins,
        });

        if let Some(h) = self.timeout_task.take() {
            h.abort();
        }
        if let Some(h) = self.abort_task.take() {
            h.abort();
        }
        self.abort_side = None;
        self.abort_deadline_ms = None;

        Some(GameFinalization {
            game_id: self.game.id,
            white_id: self.game.white_player,
            black_id: self.game.black_player,
            category: self.game.config.time_control.category(),
            rated: self.game.config.rated.is_rated(),
            moves: self.move_history.clone(),
            final_fen: Fen::from_position(&self.game.position, EnPassantMode::Legal).to_string(),
            outcome,
            reason,
        })
    }

    #[instrument(skip(self), fields(game_id = %self.game.id))]
    pub fn parse_and_apply_move(&mut self, uci: &str) -> Result<Move, MoveError> {
        let uci_move = uci
            .parse::<UciMove>()
            .map_err(|e| MoveError::InvalidUci(e.to_string()))?;
        let m = uci_move
            .to_move(&self.get_position())
            .map_err(|e| MoveError::Illegal(e.to_string()))?;
        self.make_move(m).map_err(MoveError::Illegal)?;
        Ok(m)
    }

    /// Deduct the mover's clock based on the client-reported think time,
    /// bounded by what the server considers physically possible.
    ///
    /// `think_ms` is how long the client says elapsed between rendering the
    /// position and committing the move (≈0 for a premove). We trust it, but:
    ///   - never charge more than wall-clock `elapsed` (a client can't
    ///     manufacture time by over-reporting — it only hurts itself), and
    ///   - never charge less than `elapsed - rtt_cap_ms`: the most network
    ///     latency we're willing to forgive. This floor stops a client from
    ///     under-reporting think time to bank clock. When `rtt_cap_ms` covers
    ///     the real round trip, a genuine premove's floor is ≤0 and it costs
    ///     ~0 regardless of where the player sits relative to the server.
    #[instrument(skip(self), fields(game_id = %self.game.id))]
    pub fn update_clock(&mut self, think_ms: i64, rtt_cap_ms: i64) -> Result<(), MoveError> {
        let now = Instant::now();
        let elapsed = match self.last_move_at {
            Some(t) => now.duration_since(t).as_millis() as i64,
            None => Duration::ZERO.as_millis() as i64,
        };
        // floor <= elapsed always holds (rtt_cap_ms >= 0), so clamp won't panic.
        let floor = (elapsed - rtt_cap_ms).max(0);
        let charge = think_ms.clamp(floor, elapsed);

        let mover = self.game.get_turn().other();
        let mut mover_ms = match mover {
            Color::Black => self.game.black_ms_left,
            Color::White => self.game.white_ms_left,
        };
        mover_ms -= charge;
        if let TimeMode::Increment(i) = self.game.config.time_control.mode {
            mover_ms += i;
        }
        if mover_ms <= 0 {
            return Err(MoveError::FlagFall);
        }
        match mover {
            Color::Black => self.game.black_ms_left = mover_ms,
            Color::White => self.game.white_ms_left = mover_ms,
        }
        self.last_move_at = Some(now);
        Ok(())
    }

    #[instrument(skip(self), fields(game_id = %self.game.id, %mover_id))]
    pub fn handle_move_made(
        &mut self,
        uci: String,
        mover_id: uuid::Uuid,
        think_ms: i64,
    ) -> Result<MoveOutcome, MoveError> {
        if self.current_player() != Some(mover_id) {
            return Err(MoveError::NotYourTurn);
        }
        self.parse_and_apply_move(&uci)?;
        // Forgive up to the mover's measured RTT plus a jitter margin. Falls
        // back to the bare margin if we have no RTT sample yet (e.g. a move
        // that races the first Ping).
        let rtt_cap_ms = self.rtt_of(mover_id).map(|r| r as i64).unwrap_or(0) + RTT_CAP_SLACK_MS;
        self.update_clock(think_ms, rtt_cap_ms)?;
        let sent_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        self.broadcast(GameServerMessage::MoveMade {
            uci: uci.clone(),
            white_ms_left: self.game.white_ms_left,
            black_ms_left: self.game.black_ms_left,
            turn: self.game.position.turn().into(),
            sent_at_ms,
        });

        // A move implicitly declines any pending draw offer.
        if self.draw_offer.is_some() {
            self.draw_offer = None;
            self.broadcast(GameServerMessage::DrawDecline);
        }

        // Track position for threefold repetition detection.
        let hash = self
            .get_position()
            .zobrist_hash::<Zobrist64>(EnPassantMode::Legal);
        let rep_count = {
            let c = self.position_counts.entry(hash).or_insert(0);
            *c += 1;
            *c
        };

        // Game ended on this move?
        if let Outcome::Known(known) = self.get_position().outcome() {
            let reason = match known {
                KnownOutcome::Decisive { .. } => GameOverReason::Checkmate,
                KnownOutcome::Draw => {
                    if self.get_position().is_stalemate() {
                        GameOverReason::Stalemate
                    } else {
                        GameOverReason::InsufficientMaterial
                    }
                }
            };
            let plan = self.end_game(known, reason);
            return Ok(MoveOutcome::Ended(plan));
        }
        if rep_count >= 3 {
            let plan = self.end_game(KnownOutcome::Draw, GameOverReason::Repetition);
            return Ok(MoveOutcome::Ended(plan));
        }
        if self.get_position().halfmoves() >= 100 {
            let plan = self.end_game(KnownOutcome::Draw, GameOverReason::FiftyMove);
            return Ok(MoveOutcome::Ended(plan));
        }

        let next_color = self.game.position.turn();
        let ms_until_flag = match next_color {
            Color::White => self.game.white_ms_left,
            Color::Black => self.game.black_ms_left,
        };

        self.move_history.push(uci);
        Ok(MoveOutcome::Continuing(TimeoutPlan {
            next_color,
            ms_until_flag,
        }))
    }

    /// Build a full state snapshot for sending a Resync to a single client.
    /// Mirrors the live-clock extrapolation done at WS-join time so the
    /// client's clocks land where the server thinks they should be.
    pub fn build_resync(&self) -> GameServerMessage {
        let fen = Fen::from_position(&self.game.position, EnPassantMode::Legal).to_string();
        let mut white_ms = self.game.white_ms_left;
        let mut black_ms = self.game.black_ms_left;
        let finished = matches!(self.status, GameStatus::Finished(_));
        if !finished {
            if let Some(last) = self.last_move_at {
                let elapsed = Instant::now().duration_since(last).as_millis() as i64;
                match self.game.position.turn() {
                    Color::White => white_ms = (white_ms - elapsed).max(0),
                    Color::Black => black_ms = (black_ms - elapsed).max(0),
                }
            }
        }
        let sent_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let (white_wins, black_wins) = self.session_score;
        GameServerMessage::Resync {
            position_fen: fen,
            moves: self.move_history.clone(),
            white_ms_left: white_ms,
            black_ms_left: black_ms,
            turn: self.game.position.turn().into(),
            sent_at_ms,
            clock_running: !finished && self.last_move_at.is_some(),
            white_wins,
            black_wins,
        }
    }

    /// Start the first-move abort countdown for `side`. Broadcasts the deadline
    /// to all subscribers and returns the deadline_ms so the caller can pass it
    /// to `handle_abort_timeout`. The caller is responsible for spawning the
    /// task and storing the handle on `abort_task`.
    pub fn start_abort_window(&mut self, side: Side) -> i64 {
        let deadline_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
            + 15_000;
        self.abort_side = Some(side);
        self.abort_deadline_ms = Some(deadline_ms);
        self.broadcast(GameServerMessage::AbortCountdown {
            side: Some(side),
            deadline_ms: Some(deadline_ms),
        });
        deadline_ms
    }

    /// Cancel the active abort window (called when a move is made that ends
    /// the window, not from `end_game` which cleans up inline without a
    /// superfluous clear-broadcast).
    pub fn clear_abort_window(&mut self) {
        if let Some(h) = self.abort_task.take() {
            h.abort();
        }
        self.abort_side = None;
        self.abort_deadline_ms = None;
        self.broadcast(GameServerMessage::AbortCountdown {
            side: None,
            deadline_ms: None,
        });
    }

    pub fn clear_rematch_offer(&mut self) {
        self.rematch_offer = None;
    }

    pub fn clear_draw_offer(&mut self) {
        self.draw_offer = None;
    }
}

#[instrument(skip(room, game_store), fields(?color, ms_until))]
pub async fn handle_timeout(
    room: Arc<Mutex<GameRoom>>,
    game_store: GameStore,
    color: Color,
    ms_until: i64,
) {
    if ms_until <= 0 {
        return;
    }
    tokio::time::sleep(Duration::from_millis(ms_until as u64)).await;

    let plan = {
        let mut gr = room.lock().await;

        // Bail if state changed while we slept.
        if !matches!(gr.status, GameStatus::Ongoing) {
            return;
        }
        if gr.game.position.turn() != color {
            return;
        }

        // Recompute remaining (the player might still have ms left if we slept slightly less).
        let now = Instant::now();
        let elapsed = gr
            .last_move_at
            .map(|t| now.duration_since(t).as_millis() as i64)
            .unwrap_or(0);
        let ms_left = match color {
            Color::White => gr.game.white_ms_left,
            Color::Black => gr.game.black_ms_left,
        } - elapsed;
        if ms_left > 0 {
            return;
        }

        // Flag fall confirmed.
        match color {
            Color::White => gr.game.white_ms_left = 0,
            Color::Black => gr.game.black_ms_left = 0,
        }
        let winner_color = match color {
            Color::White => Color::Black,
            Color::Black => Color::White,
        };
        gr.end_game(
            KnownOutcome::Decisive {
                winner: winner_color,
            },
            GameOverReason::Timeout,
        )
    };

    if let Some(plan) = plan {
        if let Err(e) = game_store.finalize_game(plan).await {
            tracing::warn!(?e, "finalize_game failed (timeout path)");
        }
    }
}

#[instrument(skip(room, game_store), fields(?expected_side))]
pub async fn handle_abort_timeout(
    room: Arc<Mutex<GameRoom>>,
    game_store: GameStore,
    expected_side: Color,
) {
    tokio::time::sleep(Duration::from_secs(15)).await;

    let plan = {
        let mut gr = room.lock().await;

        if !matches!(gr.status, GameStatus::Ongoing) {
            return;
        }
        // Bail if the expected side has already moved.
        if gr.game.position.turn() != expected_side {
            return;
        }
        // Defense in depth: also check move count.
        let already_moved = match expected_side {
            Color::White => !gr.move_history.is_empty(),
            Color::Black => gr.move_history.len() >= 2,
        };
        if already_moved {
            return;
        }

        gr.end_game(KnownOutcome::Draw, GameOverReason::Abort)
    };

    if let Some(plan) = plan {
        if let Err(e) = game_store.abort_game(plan).await {
            tracing::warn!(?e, "abort_game failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{GameConfig, RatingMode, TimeControl, TimeMode, Variant, Game};
    use shakmaty::{Color, KnownOutcome};
    use std::time::Duration;
    use uuid::Uuid;

    /// Minimal GameRoom with generous clocks, both players pre-connected.
    fn make_room() -> (GameRoom, Uuid, Uuid) {
        let white_id = Uuid::new_v4();
        let black_id = Uuid::new_v4();
        let config = GameConfig {
            time_control: TimeControl {
                initial_time: 600_000, // 10 min
                mode: TimeMode::Increment(0),
            },
            variant: Variant::Standard,
            rated: RatingMode::Casual,
        };
        let game = Game::new(config, white_id, black_id);
        let mut room = GameRoom::new(game, (0.0, 0.0));
        room.connected.insert(white_id, 1);
        room.connected.insert(black_id, 1);
        room.last_move_at = Some(Instant::now());
        room.status = GameStatus::Ongoing;
        (room, white_id, black_id)
    }

    /// Room with an increment time control.
    fn make_room_increment(increment_ms: i64) -> (GameRoom, Uuid, Uuid) {
        let white_id = Uuid::new_v4();
        let black_id = Uuid::new_v4();
        let config = GameConfig {
            time_control: TimeControl {
                initial_time: 600_000,
                mode: TimeMode::Increment(increment_ms),
            },
            variant: Variant::Standard,
            rated: RatingMode::Casual,
        };
        let game = Game::new(config, white_id, black_id);
        let mut room = GameRoom::new(game, (0.0, 0.0));
        room.connected.insert(white_id, 1);
        room.connected.insert(black_id, 1);
        room.last_move_at = Some(Instant::now());
        room.status = GameStatus::Ongoing;
        (room, white_id, black_id)
    }

    /// Room with no pre-connected players (WaitingForOpponent state).
    fn make_room_bare() -> (GameRoom, Uuid, Uuid) {
        let white_id = Uuid::new_v4();
        let black_id = Uuid::new_v4();
        let config = GameConfig {
            time_control: TimeControl {
                initial_time: 600_000,
                mode: TimeMode::Increment(0),
            },
            variant: Variant::Standard,
            rated: RatingMode::Casual,
        };
        let game = Game::new(config, white_id, black_id);
        let room = GameRoom::new(game, (0.0, 0.0));
        (room, white_id, black_id)
    }

    // ── threefold repetition (original) ─────────────────────────────────────

    /// Knight shuffle: Nf3 Nf6 Ng1 Ng8 repeated until the start position
    /// has appeared three times → threefold repetition draw.
    ///
    /// Position count timeline (Zobrist includes side-to-move):
    ///   seed:           start (White to move)  → count 1
    ///   g1f3 g8f6 f3g1 f6g8  → back to start  → count 2
    ///   g1f3 g8f6 f3g1 f6g8  → back to start  → count 3 → draw
    #[test]
    fn threefold_repetition_detected() {
        let (mut room, white_id, black_id) = make_room();

        let moves: &[(&str, Uuid)] = &[
            ("g1f3", white_id),
            ("g8f6", black_id),
            ("f3g1", white_id),
            ("f6g8", black_id),
            ("g1f3", white_id),
            ("g8f6", black_id),
            ("f3g1", white_id),
            ("f6g8", black_id),
        ];

        for (i, (uci, player)) in moves.iter().enumerate() {
            room.last_move_at = Some(Instant::now());
            let result = room.handle_move_made(uci.to_string(), *player, 0);
            let outcome = result.expect("move should be legal");
            if i < moves.len() - 1 {
                assert!(
                    matches!(outcome, MoveOutcome::Continuing(_)),
                    "expected game to continue after move {i}"
                );
            } else {
                assert!(
                    matches!(outcome, MoveOutcome::Ended(_)),
                    "expected game to end on move {i}"
                );
                assert_eq!(room.end_reason, Some(GameOverReason::Repetition));
            }
        }
    }

    // ── update_clock ─────────────────────────────────────────────────────────
    //
    // update_clock is called AFTER a move is applied (turn has switched).
    // get_turn().other() = the side that just moved = the clock to charge.
    // We play "e2e4" to advance past White's turn before each clock test,
    // then manipulate white_ms_left and last_move_at directly.

    #[test]
    fn clock_premove_charged_nothing() {
        let (mut room, _, _) = make_room();
        room.parse_and_apply_move("e2e4").unwrap(); // now Black to move; charges White
        room.game.white_ms_left = 60_000;
        room.last_move_at = Some(Instant::now()); // elapsed ≈ 0
        room.update_clock(0, 500).unwrap(); // think_ms=0, rtt_cap=500 → floor=0, charge=0
        // Allow up to 20ms slop for code-execution time.
        assert!(
            room.game.white_ms_left >= 59_980,
            "premove should cost near 0; left={}",
            room.game.white_ms_left
        );
    }

    #[test]
    fn clock_over_reporting_clamped_to_elapsed() {
        let (mut room, _, _) = make_room();
        room.parse_and_apply_move("e2e4").unwrap();
        room.game.white_ms_left = 60_000;
        // elapsed ≈ 100ms; think_ms = 50 000 (massive over-report)
        room.last_move_at = Some(Instant::now() - Duration::from_millis(100));
        room.update_clock(50_000, 0).unwrap(); // rtt_cap=0 → floor=elapsed; charge=elapsed
        let charged = 60_000 - room.game.white_ms_left;
        // Should be clamped to ~100ms, never 50 000ms.
        assert!(
            charged <= 200,
            "over-report must be clamped to elapsed; charged={charged}"
        );
        assert!(charged >= 80, "at least some real time should be charged; charged={charged}");
    }

    #[test]
    fn clock_under_reporting_floored() {
        let (mut room, _, _) = make_room();
        room.parse_and_apply_move("e2e4").unwrap();
        room.game.white_ms_left = 60_000;
        // elapsed ≈ 500ms; rtt_cap=200 → floor=300; think_ms=0 → charge=300
        room.last_move_at = Some(Instant::now() - Duration::from_millis(500));
        room.update_clock(0, 200).unwrap();
        let charged = 60_000 - room.game.white_ms_left;
        assert!(
            charged >= 250,
            "under-report must be floored to (elapsed-rtt_cap); charged={charged}"
        );
        assert!(charged <= 600, "shouldn't exceed elapsed+slop; charged={charged}");
    }

    #[test]
    fn clock_increment_added_after_charge() {
        let (mut room, _, _) = make_room_increment(5_000); // 5s increment
        room.parse_and_apply_move("e2e4").unwrap();
        room.game.white_ms_left = 60_000;
        room.last_move_at = Some(Instant::now()); // elapsed ≈ 0
        room.update_clock(0, 500).unwrap(); // charge ≈ 0, then +5 000
        assert!(
            room.game.white_ms_left >= 64_990,
            "increment should be added; left={}",
            room.game.white_ms_left
        );
    }

    #[test]
    fn clock_flag_fall_returns_error_and_clock_unchanged() {
        let (mut room, _, _) = make_room();
        room.parse_and_apply_move("e2e4").unwrap();
        room.game.white_ms_left = 100;
        // elapsed ≈ 500ms, no rtt forgiveness → floor=500, charge=500 > 100 → FlagFall
        room.last_move_at = Some(Instant::now() - Duration::from_millis(500));
        let result = room.update_clock(0, 0);
        assert!(
            matches!(result, Err(MoveError::FlagFall)),
            "expected FlagFall"
        );
        assert_eq!(room.game.white_ms_left, 100, "clock must not mutate on FlagFall");
    }

    // ── handle_move_made ─────────────────────────────────────────────────────

    #[test]
    fn move_not_your_turn() {
        let (mut room, _, black_id) = make_room();
        room.last_move_at = Some(Instant::now());
        // It's White's turn; Black trying to move → NotYourTurn.
        let result = room.handle_move_made("e7e5".to_string(), black_id, 0);
        assert!(matches!(result, Err(MoveError::NotYourTurn)));
    }

    #[test]
    fn move_invalid_uci_string() {
        let (mut room, white_id, _) = make_room();
        room.last_move_at = Some(Instant::now());
        let result = room.handle_move_made("notauci".to_string(), white_id, 0);
        assert!(matches!(result, Err(MoveError::InvalidUci(_))));
    }

    #[test]
    fn move_updates_history_and_returns_timeout_plan() {
        let (mut room, white_id, _) = make_room();
        room.last_move_at = Some(Instant::now());
        let outcome = room.handle_move_made("e2e4".to_string(), white_id, 0).unwrap();
        match &outcome {
            MoveOutcome::Continuing(plan) => {
                assert_eq!(plan.next_color, Color::Black);
                assert!(plan.ms_until_flag > 0);
            }
            _ => panic!("expected Continuing after e2e4"),
        }
        assert_eq!(room.move_history, vec!["e2e4"]);
    }

    /// Fool's mate: the quickest checkmate (4 moves, Black wins).
    /// f2f3 e7e5 g2g4 d8h4#
    #[test]
    fn move_checkmate_ends_game() {
        let (mut room, white_id, black_id) = make_room();
        let moves: &[(&str, Uuid)] = &[
            ("f2f3", white_id),
            ("e7e5", black_id),
            ("g2g4", white_id),
            ("d8h4", black_id), // checkmate
        ];
        for (i, (uci, player)) in moves.iter().enumerate() {
            room.last_move_at = Some(Instant::now());
            let outcome = room
                .handle_move_made(uci.to_string(), *player, 0)
                .expect("legal move");
            if i < 3 {
                assert!(matches!(outcome, MoveOutcome::Continuing(_)));
            } else {
                assert!(
                    matches!(outcome, MoveOutcome::Ended(_)),
                    "expected game to end on checkmate"
                );
                assert_eq!(room.end_reason, Some(GameOverReason::Checkmate));
            }
        }
    }

    #[test]
    fn move_clears_pending_draw_offer() {
        let (mut room, white_id, black_id) = make_room();
        room.draw_offer = Some(black_id); // Black offered a draw
        room.last_move_at = Some(Instant::now());
        // White makes a move → implicitly declines the draw offer.
        room.handle_move_made("e2e4".to_string(), white_id, 0).unwrap();
        assert!(room.draw_offer.is_none(), "draw offer should be cleared after a move");
    }

    // ── end_game ─────────────────────────────────────────────────────────────

    #[test]
    fn end_game_idempotent_returns_none_on_second_call() {
        let (mut room, _, _) = make_room();
        let plan1 = room.end_game(
            KnownOutcome::Decisive { winner: Color::White },
            GameOverReason::Checkmate,
        );
        assert!(plan1.is_some(), "first call should return a finalization plan");
        assert!(matches!(room.status, GameStatus::Finished(_)));

        let plan2 = room.end_game(
            KnownOutcome::Decisive { winner: Color::Black },
            GameOverReason::Resignation,
        );
        assert!(plan2.is_none(), "second call should be a no-op");
        // Session score should only reflect the first outcome.
        assert_eq!(room.session_score.0, 1.0, "only one white win should be counted");
        assert_eq!(room.session_score.1, 0.0);
    }

    #[test]
    fn end_game_session_score_white_win() {
        let (mut room, _, _) = make_room();
        room.end_game(
            KnownOutcome::Decisive { winner: Color::White },
            GameOverReason::Checkmate,
        );
        assert_eq!(room.session_score, (1.0, 0.0));
    }

    #[test]
    fn end_game_session_score_black_win() {
        let (mut room, _, _) = make_room();
        room.end_game(
            KnownOutcome::Decisive { winner: Color::Black },
            GameOverReason::Resignation,
        );
        assert_eq!(room.session_score, (0.0, 1.0));
    }

    #[test]
    fn end_game_session_score_draw() {
        let (mut room, _, _) = make_room();
        room.end_game(KnownOutcome::Draw, GameOverReason::Stalemate);
        assert_eq!(room.session_score, (0.5, 0.5));
    }

    #[test]
    fn end_game_finalization_has_correct_players() {
        let (mut room, white_id, black_id) = make_room();
        let plan = room
            .end_game(
                KnownOutcome::Decisive { winner: Color::White },
                GameOverReason::Checkmate,
            )
            .unwrap();
        assert_eq!(plan.game_id, room.game.id);
        assert_eq!(plan.white_id, white_id);
        assert_eq!(plan.black_id, black_id);
    }

    // ── add_player / remove_player / current_player ──────────────────────────

    #[test]
    fn add_both_players_transitions_to_ongoing() {
        let (mut room, white_id, black_id) = make_room_bare();
        assert!(matches!(room.status, GameStatus::WaitingForOpponent));

        let (role_w, started) = room.add_player(white_id);
        assert!(matches!(role_w, shared::PlayerRole::Player(shared::Side::White)));
        assert!(!started, "game not started with only one player");
        assert!(matches!(room.status, GameStatus::WaitingForOpponent));

        let (role_b, started) = room.add_player(black_id);
        assert!(matches!(role_b, shared::PlayerRole::Player(shared::Side::Black)));
        assert!(started, "game should start when second player joins");
        assert!(matches!(room.status, GameStatus::Ongoing));
    }

    #[test]
    fn add_player_reconnect_does_not_restart() {
        let (mut room, white_id, black_id) = make_room_bare();
        room.add_player(white_id);
        room.add_player(black_id); // game starts, history still empty
        // White disconnects then reconnects.
        room.remove_player(white_id);
        let (_, restarted) = room.add_player(white_id);
        assert!(!restarted, "reconnect must not trigger started=true");
    }

    #[test]
    fn add_player_spectator_not_tracked() {
        let (mut room, _, _) = make_room_bare();
        let spectator = Uuid::new_v4();
        let (role, started) = room.add_player(spectator);
        assert!(matches!(role, shared::PlayerRole::Spectator));
        assert!(!started);
        assert!(!room.connected.contains_key(&spectator));
    }

    #[test]
    fn remove_player_refcount_multi_session() {
        let (mut room, white_id, _) = make_room_bare();
        room.add_player(white_id); // count → 1
        room.add_player(white_id); // count → 2
        room.remove_player(white_id); // count → 1, still connected
        assert!(room.connected.contains_key(&white_id));
        room.remove_player(white_id); // count → 0, disconnected
        assert!(!room.connected.contains_key(&white_id));
    }

    #[test]
    fn current_player_none_when_side_to_move_is_disconnected() {
        let (mut room, white_id, _) = make_room_bare();
        // Only White connected; it's White's turn → Some(white_id).
        room.add_player(white_id);
        assert_eq!(room.current_player(), Some(white_id));
        room.remove_player(white_id);
        assert_eq!(room.current_player(), None);
    }
}
