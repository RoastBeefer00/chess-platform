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
    /// Remaining time as of `sent_at_ms`, in ms. `None` for a game owned by
    /// another instance — clocks aren't (yet) mirrored into the
    /// cross-instance Redis roster, only positions are.
    pub white_ms_left: Option<i64>,
    pub black_ms_left: Option<i64>,
    /// Server time (UNIX epoch ms) `white_ms_left`/`black_ms_left` were
    /// true as of — paired with the FEN's side-to-move, this is what lets
    /// the client tick the mover's clock down locally between updates, the
    /// same snapshot+timestamp idiom `Clock` already uses everywhere else.
    pub sent_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WatchServerMessage {
    /// Full roster snapshot — sent on connect and on every refresh tick.
    Roster {
        games: Vec<WatchGameSummary>,
        total_active: usize,
    },
    /// Live position + clock update for one game already in the roster.
    /// Only ever sent for a locally-owned game (see `watch_websocket`), so
    /// unlike `WatchGameSummary`'s the clocks here are never `None`.
    Position {
        game_id: Uuid,
        fen: String,
        white_ms_left: i64,
        black_ms_left: i64,
        sent_at_ms: i64,
    },
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
                white_ms_left: Some(180_000),
                black_ms_left: None,
                sent_at_ms: 1_700_000_000_000,
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
        assert_eq!(games[0].white_ms_left, Some(180_000));
        assert_eq!(games[0].black_ms_left, None);
    }

    #[test]
    fn position_roundtrip() {
        let msg = WatchServerMessage::Position {
            game_id: Uuid::new_v4(),
            fen: "8/8/8/8/8/8/8/8 w - - 0 1".to_string(),
            white_ms_left: 150_000,
            black_ms_left: 120_000,
            sent_at_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let back: WatchServerMessage = serde_json::from_str(&json).expect("deserialize");
        match back {
            WatchServerMessage::Position { fen, white_ms_left, black_ms_left, .. } => {
                assert_eq!(fen, "8/8/8/8/8/8/8/8 w - - 0 1");
                assert_eq!(white_ms_left, 150_000);
                assert_eq!(black_ms_left, 120_000);
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
