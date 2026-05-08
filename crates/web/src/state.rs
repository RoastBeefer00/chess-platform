use std::{collections::HashMap, sync::Arc};

use axum::extract::FromRef;
use leptos::config::LeptosOptions;
use shared::Game;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::game_room::GameRoom;

pub type GameId = Uuid;
pub type GameRooms = Arc<Mutex<HashMap<GameId, GameRoom>>>;

#[derive(Clone)]
pub struct AppState {
    pub leptos_options: LeptosOptions,
    pub games: GameRooms,
}

impl FromRef<AppState> for LeptosOptions {
    fn from_ref(state: &AppState) -> Self {
        state.leptos_options.clone()
    }
}

impl AppState {
    pub fn new(leptos_options: LeptosOptions) -> Self {
        AppState {
            leptos_options,
            games: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn create_game(&self, white_player: Uuid, black_player: Uuid) -> GameId {
        let game = GameRoom::new(Game::new(white_player, black_player));
        let game_id = game.game.id;
        let mut games = self.games.blocking_lock();
        games.insert(game_id, game);
        game_id
    }

    pub fn player_count(&self) -> usize {
        let games = self.games.blocking_lock();
        games.values().map(|room| room.player_count()).sum()
    }
}
