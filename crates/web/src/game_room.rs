use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use shakmaty::{uci::UciMove, Chess, Color, KnownOutcome, Move, Outcome, Position as _};
use tokio::{
    sync::{
        broadcast::{self, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};
use tracing::instrument;

use shared::{messages::GameOverReason, Game, GameServerMessage, GameStatus, PlayerRole, Side};
use uuid::Uuid;

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
    Ended,
}

#[derive(Debug)]
pub struct GameRoom {
    pub game: Game,
    pub status: GameStatus,
    connected: HashSet<Uuid>,
    tx: Sender<GameServerMessage>,
    pub last_move_at: Option<Instant>,
    pub timeout_task: Option<JoinHandle<()>>,
}

impl GameRoom {
    pub fn new(game: Game) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        GameRoom {
            game,
            status: GameStatus::WaitingForOpponent,
            connected: HashSet::new(),
            tx,
            last_move_at: None,
            timeout_task: None,
        }
    }

    pub fn subscribe(&self) -> Receiver<GameServerMessage> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, msg: GameServerMessage) {
        let _ = self.tx.send(msg);
    }

    pub fn player_count(&self) -> usize {
        self.connected.len()
    }

    pub fn get_position(&self) -> Chess {
        self.game.position.clone()
    }

    #[instrument(skip(self))]
    pub fn add_player(&mut self, player_id: Uuid) -> PlayerRole {
        let role = if player_id == self.game.white_player {
            self.connected.insert(player_id);
            PlayerRole::Player(Color::White.into())
        } else if player_id == self.game.black_player {
            self.connected.insert(player_id);
            PlayerRole::Player(Color::Black.into())
        } else {
            PlayerRole::Spectator
        };

        if self.connected.contains(&self.game.white_player)
            && self.connected.contains(&self.game.black_player)
        {
            self.status = GameStatus::Ongoing;
        }

        role
    }

    #[instrument(skip(self))]
    pub fn remove_player(&mut self, id: Uuid) {
        self.connected.remove(&id);
    }

    pub fn current_player(&self) -> Option<Uuid> {
        let id = match self.game.position.turn() {
            Color::White => self.game.white_player,
            Color::Black => self.game.black_player,
        };
        // Only consider it "their turn" if they're actually connected.
        self.connected.contains(&id).then_some(id)
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

    pub fn end_game(&mut self, outcome: KnownOutcome, reason: GameOverReason) {
        self.status = GameStatus::Finished(Outcome::Known(outcome));
        let winner = match outcome {
            KnownOutcome::Decisive { winner } => Some(Side::from(winner)),
            KnownOutcome::Draw => None,
        };
        self.broadcast(GameServerMessage::GameOver { winner, reason });
        if let Some(h) = self.timeout_task.take() {
            h.abort();
        }
    }

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

    pub fn update_clock(&mut self) -> Result<(), MoveError> {
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
        mover_ms -= elapsed;
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

    pub fn handle_move_made(
        &mut self,
        uci: String,
        mover_id: uuid::Uuid,
    ) -> Result<MoveOutcome, MoveError> {
        if self.current_player() != Some(mover_id) {
            return Err(MoveError::NotYourTurn);
        }
        self.parse_and_apply_move(&uci)?;
        self.update_clock()?;
        let sent_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        self.broadcast(GameServerMessage::MoveMade {
            uci,
            white_ms_left: self.game.white_ms_left,
            black_ms_left: self.game.black_ms_left,
            turn: self.game.position.turn().into(),
            sent_at_ms,
        });

        // Game ended on this move?
        if let Outcome::Known(known) = self.get_position().outcome() {
            let reason = match known {
                KnownOutcome::Decisive { .. } => GameOverReason::Checkmate,
                KnownOutcome::Draw => GameOverReason::Draw,
            };
            self.end_game(known, reason);
            return Ok(MoveOutcome::Ended);
        }

        let next_color = self.game.position.turn();
        let ms_until_flag = match next_color {
            Color::White => self.game.white_ms_left,
            Color::Black => self.game.black_ms_left,
        };
        Ok(MoveOutcome::Continuing(TimeoutPlan {
            next_color,
            ms_until_flag,
        }))
    }
}

pub async fn handle_timeout(room: Arc<Mutex<GameRoom>>, color: Color, ms_until: i64) {
    if ms_until <= 0 {
        return;
    }
    tokio::time::sleep(Duration::from_millis(ms_until as u64)).await;

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
    );
}
