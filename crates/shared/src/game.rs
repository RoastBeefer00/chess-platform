use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Color, Move, Outcome, PlayError, Position as _};
use uuid::Uuid;

use crate::GameConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub id: Uuid,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub rating: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub id: Uuid,
    pub white: PlayerInfo,
    pub black: PlayerInfo,
    pub white_ms_left: i64,
    pub black_ms_left: i64,
    pub sent_at_ms: i64,
    pub clock_running: bool,
    pub config: GameConfig,
}

#[derive(Debug, Clone)]
pub enum GameStatus {
    Ongoing,
    Finished(Outcome),
    WaitingForOpponent,
}

#[derive(Debug, Clone)]
pub struct Game {
    pub id: Uuid,
    pub config: GameConfig,
    pub position: Chess,
    pub white_player: Uuid,
    pub black_player: Uuid,
    pub white_ms_left: i64,
    pub black_ms_left: i64,
}

impl Game {
    pub fn new(config: GameConfig, white_player: Uuid, black_player: Uuid) -> Self {
        let pos = Chess::default();
        Game {
            id: Uuid::new_v4(),
            config: config.clone(),
            position: pos,
            white_player,
            black_player,
            white_ms_left: config.time_control.initial_time,
            black_ms_left: config.time_control.initial_time,
        }
    }

    /// Attempts to play a move on the current position.
    ///
    /// Returns `Ok(Outcome)` on success or `Err(PlayError)` if the move is illegal.
    pub fn make_move(&mut self, r#move: Move) -> Result<Outcome, PlayError<Chess>> {
        self.position = self.position.clone().play(r#move)?;
        Ok(self.position.outcome())
    }

    pub fn get_turn(&self) -> Color {
        self.position.turn()
    }
}
