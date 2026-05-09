use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome { username: String },
    UserLeft { username: String },
    MoveMade { uci: String },
    Chat { user: Uuid, text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMessage {
    UserJoined { uuid: Uuid },
    UserLeft { uuid: Uuid },
    MoveMade { uci: String },
    Chat { user: Uuid, text: String },
}
