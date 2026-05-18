use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::PlayerRole;

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
    },
    Chat {
        user: Uuid,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameClientMessage {
    UserJoined { game_id: Uuid },
    MoveMade { uci: String },
    Chat { text: String },
}
