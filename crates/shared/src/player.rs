use shakmaty::Color;

pub enum PlayerRole {
    Player(Color),
    Spectator,
}

impl PlayerRole {
    pub fn color(&self) -> Option<Color> {
        match self {
            PlayerRole::Player(color) => Some(*color),
            PlayerRole::Spectator => None,
        }
    }
}
