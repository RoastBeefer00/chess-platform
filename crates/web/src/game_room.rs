use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use shakmaty::{
    fen::Fen, uci::UciMove, Chess, Color, EnPassantMode, KnownOutcome, Move, Outcome, Position as _,
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
    connected: HashMap<Uuid, u32>,
    tx: Sender<GameServerMessage>,
    pub last_move_at: Option<Instant>,
    pub timeout_task: Option<JoinHandle<()>>,
    pub rematch_offer: Option<Uuid>,
    pub draw_offer: Option<Uuid>,
    pub move_history: Vec<String>,
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
        GameRoom {
            game,
            status: GameStatus::WaitingForOpponent,
            connected: HashMap::new(),
            tx,
            last_move_at: None,
            timeout_task: None,
            rematch_offer: None,
            draw_offer: None,
            move_history: Vec::new(),
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

    pub fn get_position(&self) -> Chess {
        self.game.position.clone()
    }

    #[instrument(skip(self))]
    pub fn add_player(&mut self, player_id: Uuid) -> PlayerRole {
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

        role
    }

    /// Decrement the session count for this user; remove the entry when it
    /// hits zero. Safe to call for unknown UUIDs (spectators) — it's a no-op.
    #[instrument(skip(self))]
    pub fn remove_player(&mut self, id: Uuid) {
        if let Some(count) = self.connected.get_mut(&id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.connected.remove(&id);
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
            KnownOutcome::Decisive { winner: Color::White } => self.session_score.0 += 1.0,
            KnownOutcome::Decisive { winner: Color::Black } => self.session_score.1 += 1.0,
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
            is_stalemate: self.game.position.is_stalemate(),
            is_insufficient_material: self.game.position.is_insufficient_material(),
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

    #[instrument(skip(self), fields(game_id = %self.game.id))]
    pub fn update_clock(&mut self, lag_ms: i64) -> Result<(), MoveError> {
        let now = Instant::now();
        let elapsed = match self.last_move_at {
            Some(t) => now.duration_since(t).as_millis() as i64,
            None => Duration::ZERO.as_millis() as i64,
        };
        let mover = self.game.get_turn().other();
        let mut mover_ms = match mover {
            Color::Black => self.game.black_ms_left,
            Color::White => self.game.white_ms_left,
        };
        mover_ms -= (elapsed - lag_ms).max(0);
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
        lag_ms: i64,
    ) -> Result<MoveOutcome, MoveError> {
        if self.current_player() != Some(mover_id) {
            return Err(MoveError::NotYourTurn);
        }
        self.parse_and_apply_move(&uci)?;
        self.update_clock(lag_ms)?;
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

        // Game ended on this move?
        if let Outcome::Known(known) = self.get_position().outcome() {
            let reason = match known {
                KnownOutcome::Decisive { .. } => GameOverReason::Checkmate,
                KnownOutcome::Draw => GameOverReason::Draw,
            };
            let plan = self.end_game(known, reason);
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
