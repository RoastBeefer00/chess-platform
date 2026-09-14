use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Category, PlayerInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchGameSummary {
    pub game_id: Uuid,
    pub white: PlayerInfo,
    pub black: PlayerInfo,
    pub category: Category,
    pub rated: bool,
    pub fen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WatchServerMessage {
    /// Full roster snapshot — sent on connect and on every refresh tick.
    Roster {
        games: Vec<WatchGameSummary>,
        total_active: usize,
    },
    /// Live position update for one game already in the roster.
    Position { game_id: Uuid, fen: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WatchClientMessage {
    Connect,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(name: &str) -> PlayerInfo {
        PlayerInfo {
            id: Uuid::new_v4(),
            username: Some(name.to_string()),
            avatar_url: None,
            rating: 1500,
        }
    }

    #[test]
    fn roster_roundtrip() {
        let msg = WatchServerMessage::Roster {
            games: vec![WatchGameSummary {
                game_id: Uuid::new_v4(),
                white: player("white"),
                black: player("black"),
                category: Category::Blitz,
                rated: true,
                fen: "startpos".to_string(),
            }],
            total_active: 5,
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let WatchServerMessage::Roster { games, total_active } =
            serde_json::from_str(&json).expect("deserialize")
        else {
            panic!("expected Roster");
        };
        assert_eq!(games.len(), 1);
        assert_eq!(total_active, 5);
        assert_eq!(games[0].white.username.as_deref(), Some("white"));
    }

    #[test]
    fn position_roundtrip() {
        let msg = WatchServerMessage::Position {
            game_id: Uuid::new_v4(),
            fen: "8/8/8/8/8/8/8/8 w - - 0 1".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let back: WatchServerMessage = serde_json::from_str(&json).expect("deserialize");
        match back {
            WatchServerMessage::Position { fen, .. } => {
                assert_eq!(fen, "8/8/8/8/8/8/8/8 w - - 0 1")
            }
            _ => panic!("expected Position"),
        }
    }

    #[test]
    fn client_connect_roundtrip() {
        let json = serde_json::to_string(&WatchClientMessage::Connect).expect("serialize");
        let _: WatchClientMessage = serde_json::from_str(&json).expect("deserialize");
    }
}
