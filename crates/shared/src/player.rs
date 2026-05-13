use serde::{Deserialize, Serialize};
use shakmaty::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    White,
    Black,
}

impl From<Color> for Side {
    fn from(color: Color) -> Self {
        match color {
            Color::White => Side::White,
            Color::Black => Side::Black,
        }
    }
}

impl From<Side> for Color {
    fn from(side: Side) -> Self {
        match side {
            Side::White => Color::White,
            Side::Black => Color::Black,
        }
    }
}

impl From<Color> for PlayerRole {
    fn from(color: Color) -> Self {
        PlayerRole::Player(color.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerRole {
    Player(Side),
    Spectator,
}

impl PlayerRole {
    pub fn color(&self) -> Option<Color> {
        match self {
            PlayerRole::Player(side) => Some((*side).into()),
            PlayerRole::Spectator => None,
        }
    }
}
