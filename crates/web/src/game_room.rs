use shakmaty::{Chess, Color, Move, Position as _};
use tokio::sync::broadcast::{self, Receiver, Sender};
use tracing::instrument;

use shared::{Game, GameServerMessage, GameStatus, PlayerRole};
use uuid::Uuid;

const BROADCAST_CAPACITY: usize = 32;

#[derive(Debug)]
pub struct GameRoom {
    pub game: Game,
    pub white_player: Option<Uuid>,
    pub black_player: Option<Uuid>,
    pub status: GameStatus,
    tx: Sender<GameServerMessage>,
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

    pub fn subscribe(&self) -> Receiver<GameServerMessage> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, msg: GameServerMessage) {
        let _ = self.tx.send(msg);
    }

    pub fn player_count(&self) -> usize {
        self.white_player.is_some() as usize + self.black_player.is_some() as usize
    }

    pub fn get_position(&self) -> Chess {
        self.game.position.clone()
    }

    #[instrument(skip(self))]
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
            PlayerRole::Player(color.into())
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
        } else if self.black_player.is_some_and(|id| id == player_id) {
            PlayerRole::Player(Color::Black.into())
        } else if self.white_player.is_some_and(|id| id == player_id) {
            PlayerRole::Player(Color::White.into())
        } else if self.white_player.is_none() {
            assign_player(self, GameStatus::Ongoing, player_id, Color::White)
        } else if self.black_player.is_none() {
            assign_player(self, GameStatus::Ongoing, player_id, Color::Black)
        } else {
            PlayerRole::Spectator
        }
    }

    #[instrument(skip(self))]
    pub fn remove_player(&mut self, id: Uuid) {
        if self.white_player == Some(id) {
            self.white_player = None;
        } else if self.black_player == Some(id) {
            self.black_player = None;
        }
    }

    pub fn current_player(&self) -> Option<Uuid> {
        match self.game.position.turn() {
            Color::White => self.white_player,
            Color::Black => self.black_player,
        }
    }

    #[instrument(skip(self))]
    pub fn make_move(&mut self, mv: Move) -> Result<(), String> {
        let chess = self.get_position();
        match chess.play(mv) {
            Ok(pos) => {
                self.game.position = pos;
                Ok(())
            }
            Err(_) => Err("Illegal move".to_string()),
        }
    }
}
