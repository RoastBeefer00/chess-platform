use shakmaty::{Chess, Color, Move, Outcome, PlayError, Position};
use uuid::Uuid;

pub struct Game {
    id: Uuid,
    position: Chess,
    white_player: Uuid,
    black_player: Uuid,
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

    pub fn make_move(&mut self, r#move: &Move) -> Result<Option<Outcome>, PlayError<Chess>> {
        self.position = self.position.clone().play(r#move)?;
        Ok(self.position.outcome())
    }

    pub fn get_turn(&self) -> Color {
        self.position.turn()
    }
}
