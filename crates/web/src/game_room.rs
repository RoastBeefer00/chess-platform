use shakmaty::{Chess, Color, Position};
use tokio::sync::broadcast::{self, Receiver, Sender};

use shared::{Game, GameStatus, PlayerRole, ServerMessage};
use uuid::Uuid;

const BROADCAST_CAPACITY: usize = 32;

#[derive(Debug)]
pub struct GameRoom {
    pub game: Game,
    pub white_player: Option<Uuid>,
    pub black_player: Option<Uuid>,
    pub status: GameStatus,
    tx: Sender<ServerMessage>,
}

impl GameRoom {
    pub fn new(game: Game) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        GameRoom {
            game,
            white_player: None,
            black_player: None,
            status: GameStatus::WaitingForOpponent,
            tx,
        }
    }

    pub fn subscribe(&self) -> Receiver<ServerMessage> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, msg: ServerMessage) {
        let _ = self.tx.send(msg);
    }

    pub fn player_count(&self) -> usize {
        self.white_player.is_some() as usize + self.black_player.is_some() as usize
    }

    pub fn get_position(&self) -> Chess {
        self.game.position.clone()
    }

    pub fn add_player(&mut self, player_id: Uuid) -> PlayerRole {
        fn assign_player(
            room: &mut GameRoom,
            status: GameStatus,
            player_id: Uuid,
            color: Color,
        ) -> PlayerRole {
            match color {
                Color::White => room.white_player = Some(player_id),
                Color::Black => room.black_player = Some(player_id),
            }
            room.status = status;
            PlayerRole::Player(color)
        }

        if self.white_player.is_none() && self.black_player.is_none() {
            // Randomly assign the first player to white or black
            if rand::random() {
                assign_player(
                    self,
                    GameStatus::WaitingForOpponent,
                    player_id,
                    Color::White,
                )
            } else {
                assign_player(
                    self,
                    GameStatus::WaitingForOpponent,
                    player_id,
                    Color::Black,
                )
            }
        } else if self.white_player.is_none() {
            assign_player(self, GameStatus::Ongoing, player_id, Color::White)
        } else if self.black_player.is_none() {
            assign_player(self, GameStatus::Ongoing, player_id, Color::Black)
        } else {
            PlayerRole::Spectator
        }
    }
}
