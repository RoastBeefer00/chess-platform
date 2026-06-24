use serde::{Deserialize, Serialize};
use shakmaty::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    White,
    Black,
}

impl Side {
    pub fn opposite(&self) -> Side {
        match self {
            Side::White => Side::Black,
            Side::Black => Side::White,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::Color;

    #[test]
    fn side_opposite_involution() {
        assert_eq!(Side::White.opposite(), Side::Black);
        assert_eq!(Side::Black.opposite(), Side::White);
        assert_eq!(Side::White.opposite().opposite(), Side::White);
        assert_eq!(Side::Black.opposite().opposite(), Side::Black);
    }

    #[test]
    fn color_to_side_round_trip() {
        assert_eq!(Side::from(Color::White), Side::White);
        assert_eq!(Side::from(Color::Black), Side::Black);
    }

    #[test]
    fn side_to_color_round_trip() {
        assert_eq!(Color::from(Side::White), Color::White);
        assert_eq!(Color::from(Side::Black), Color::Black);
    }

    #[test]
    fn color_side_color_round_trip() {
        for color in [Color::White, Color::Black] {
            assert_eq!(Color::from(Side::from(color)), color);
        }
    }

    #[test]
    fn color_to_player_role() {
        let role_w = PlayerRole::from(Color::White);
        let role_b = PlayerRole::from(Color::Black);
        assert_eq!(role_w, PlayerRole::Player(Side::White));
        assert_eq!(role_b, PlayerRole::Player(Side::Black));
    }

    #[test]
    fn player_role_color_some_for_players() {
        assert_eq!(PlayerRole::Player(Side::White).color(), Some(Color::White));
        assert_eq!(PlayerRole::Player(Side::Black).color(), Some(Color::Black));
    }

    #[test]
    fn player_role_color_none_for_spectator() {
        assert_eq!(PlayerRole::Spectator.color(), None);
    }
}
