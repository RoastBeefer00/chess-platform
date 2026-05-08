use tokio::sync::mpsc::Sender;

use shared::{Game, GameStatus, ServerMessage};

pub struct GameRoom {
    pub game: Game,
    pub white_tx: Option<Sender<ServerMessage>>,
    pub black_tx: Option<Sender<ServerMessage>>,
    pub spectator_txs: Vec<Sender<ServerMessage>>,
    pub status: GameStatus,
}

impl GameRoom {
    pub fn new(game: Game) -> Self {
        GameRoom {
            game,
            white_tx: None,
            black_tx: None,
            spectator_txs: Vec::new(),
            status: GameStatus::WaitingForOpponent,
        }
    }

    pub async fn broadcast(&self, msg: ServerMessage) {
        for tx in self
            .white_tx
            .iter()
            .chain(self.black_tx.iter())
            .chain(self.spectator_txs.iter())
        {
            let _ = tx.send(msg.clone()).await;
        }
    }

    pub fn player_count(&self) -> usize {
        self.white_tx.is_some() as usize + self.black_tx.is_some() as usize
    }
}
