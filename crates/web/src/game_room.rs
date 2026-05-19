use std::collections::HashSet;

use shakmaty::{Chess, Color, Move, Position as _};
use tokio::sync::broadcast::{self, Receiver, Sender};
use tracing::instrument;

use shared::{Game, GameServerMessage, GameStatus, PlayerRole};
use uuid::Uuid;

const BROADCAST_CAPACITY: usize = 32;

#[derive(Debug)]
pub struct GameRoom {
    pub game: Game,
    pub status: GameStatus,
    connected: HashSet<Uuid>,
    tx: Sender<GameServerMessage>,
}

impl GameRoom {
    pub fn new(game: Game) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        GameRoom {
            game,
            status: GameStatus::WaitingForOpponent,
            connected: HashSet::new(),
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
        self.connected.len()
    }

    pub fn get_position(&self) -> Chess {
        self.game.position.clone()
    }

    #[instrument(skip(self))]
    pub fn add_player(&mut self, player_id: Uuid) -> PlayerRole {
        let role = if player_id == self.game.white_player {
            self.connected.insert(player_id);
            PlayerRole::Player(Color::White.into())
        } else if player_id == self.game.black_player {
            self.connected.insert(player_id);
            PlayerRole::Player(Color::Black.into())
        } else {
            PlayerRole::Spectator
        };

        if self.connected.contains(&self.game.white_player)
            && self.connected.contains(&self.game.black_player)
        {
            self.status = GameStatus::Ongoing;
        }

        role
    }

    #[instrument(skip(self))]
    pub fn remove_player(&mut self, id: Uuid) {
        self.connected.remove(&id);
    }

    pub fn current_player(&self) -> Option<Uuid> {
        let id = match self.game.position.turn() {
            Color::White => self.game.white_player,
            Color::Black => self.game.black_player,
        };
        // Only consider it "their turn" if they're actually connected.
        self.connected.contains(&id).then_some(id)
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
