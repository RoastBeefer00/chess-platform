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
    /// Parse a UCI string and play the move on this game.
    /// Convenience wrapper used in tests and the server crate.
    #[cfg(test)]
    fn play_uci(&mut self, uci: &str) -> Result<Outcome, Box<dyn std::error::Error>> {
        use shakmaty::uci::UciMove;
        let uci_move: UciMove = uci.parse()?;
        let mv = uci_move.to_move(&self.position)?;
        Ok(self.make_move(mv)?)
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GameConfig, RatingMode, TimeControl, TimeMode, Variant};
    use shakmaty::{Color, Outcome};
    use uuid::Uuid;

    fn make_game() -> Game {
        let config = GameConfig {
            time_control: TimeControl {
                initial_time: 600_000,
                mode: TimeMode::Increment(0),
            },
            variant: Variant::Standard,
            rated: RatingMode::Casual,
        };
        Game::new(config, Uuid::new_v4(), Uuid::new_v4())
    }

    #[test]
    fn new_game_clocks_equal_initial_time() {
        let game = make_game();
        assert_eq!(game.white_ms_left, 600_000);
        assert_eq!(game.black_ms_left, 600_000);
    }

    #[test]
    fn new_game_starts_with_white_to_move() {
        let game = make_game();
        assert_eq!(game.get_turn(), Color::White);
    }

    #[test]
    fn legal_move_advances_turn() {
        let mut game = make_game();
        game.play_uci("e2e4").unwrap();
        assert_eq!(game.get_turn(), Color::Black, "after White moves, Black to move");
    }

    #[test]
    fn legal_move_ongoing_returns_none_outcome() {
        let mut game = make_game();
        let outcome = game.play_uci("e2e4").unwrap();
        assert_eq!(outcome, Outcome::Unknown, "game is still ongoing");
    }

    #[test]
    fn illegal_move_returns_err_and_position_unchanged() {
        use shakmaty::{uci::UciMove, Position as _};
        let mut game = make_game();
        let pos_before = game.position.clone();
        // e2e5 is not a legal pawn move.
        let uci_move: UciMove = "e2e5".parse().unwrap();
        let mv_result = uci_move.to_move(&game.position);
        if let Ok(mv) = mv_result {
            let result = game.make_move(mv);
            assert!(result.is_err(), "illegal move must return Err");
        }
        // Either parsing or playing failed; position must be unchanged.
        assert_eq!(
            game.position.board(),
            pos_before.board(),
            "position must not change on illegal move"
        );
        assert_eq!(game.get_turn(), Color::White, "turn must not advance");
    }

    /// Fool's mate: f2f3 e7e5 g2g4 d8h4# → Black wins by checkmate.
    #[test]
    fn checkmate_returns_decisive_outcome() {
        let mut game = make_game();
        game.play_uci("f2f3").unwrap();
        game.play_uci("e7e5").unwrap();
        game.play_uci("g2g4").unwrap();
        let outcome = game.play_uci("d8h4").unwrap();
        assert_eq!(
            outcome,
            Outcome::Known(shakmaty::KnownOutcome::Decisive { winner: Color::Black })
        );
    }
}
