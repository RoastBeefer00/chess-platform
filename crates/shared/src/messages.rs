use shakmaty::Move;
use uuid::Uuid;

pub enum ServerMessage {
    Welcome { username: String },
    UserLeft { username: String },
    MoveMade { r#move: Move },
    Chat { user: Uuid, text: String },
}

pub enum ClientMessage {
    UserJoined { uuid: Uuid },
    UserLeft { uuid: Uuid },
    MoveMade { r#move: Move },
    Chat { user: Uuid, text: String },
}
