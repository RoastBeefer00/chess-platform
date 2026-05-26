use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{PlayerRole, Side};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameOverReason {
    Abort,
    Checkmate,
    Draw,
    Timeout,
    Resignation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameServerMessage {
    UserJoined {
        uuid: Uuid,
        position_fen: String,
        player_role: PlayerRole,
        moves: Vec<String>,
    },
    UserLeft {
        username: String,
    },
    MoveMade {
        uci: String,
        white_ms_left: i64,
        black_ms_left: i64,
        turn: Side,
        sent_at_ms: i64,
    },
    Chat {
        user: Uuid,
        text: String,
    },
    ClockSync {
        white_ms_left: i64,
        black_ms_left: i64,
        turn: Side,
        sent_at_ms: i64,
        clock_running: bool,
    },
    /// Authoritative snapshot. Client replaces local position/history/clocks
    /// with these values. Sent when the server rejects a client move or
    /// detects the client has fallen behind (broadcast lag), so the client
    /// can recover without a page refresh.
    Resync {
        position_fen: String,
        moves: Vec<String>,
        white_ms_left: i64,
        black_ms_left: i64,
        turn: Side,
        sent_at_ms: i64,
        clock_running: bool,
    },
    GameOver {
        winner: Option<Side>,
        reason: GameOverReason,
    },
    RematchOffer {
        from: Uuid,
    },
    RematchAccept {
        new_game_id: Uuid,
    },
    RematchDecline,
    RematchCancel,
    DrawOffer {
        from: Uuid,
    },
    DrawDecline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameClientMessage {
    UserJoined { game_id: Uuid },
    MoveMade { uci: String },
    Chat { text: String },
    Resign,
    DrawOffer,
    DrawAccept,
    DrawDecline,
    RematchOffer,
    RematchAccept,
    RematchDecline,
    RematchCancel,
}
