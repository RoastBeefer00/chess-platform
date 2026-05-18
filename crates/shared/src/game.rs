use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Color, Move, Outcome, PlayError, Position as _};
use uuid::Uuid;

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
    pub position: Chess,
    pub white_player: Uuid,
    pub black_player: Uuid,
}

impl Game {
    pub fn new(white_player: Uuid, black_player: Uuid) -> Self {
        let pos = Chess::default();
        Game {
            id: Uuid::new_v4(),
            position: pos,
            white_player,
            black_player,
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
