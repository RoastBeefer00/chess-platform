use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{PlayerRole, Side};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameOverReason {
    Abort,
    Checkmate,
    Draw,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameServerMessage {
    UserJoined {
        uuid: Uuid,
        position_fen: String,
        player_role: PlayerRole,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameClientMessage {
    UserJoined { game_id: Uuid },
    MoveMade { uci: String },
    Chat { text: String },
    RematchOffer,
    RematchAccept,
    RematchDecline,
    RematchCancel,
}
