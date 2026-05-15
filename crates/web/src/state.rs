use std::{collections::HashMap, sync::Arc};

use axum::extract::FromRef;
use leptos::config::LeptosOptions;
use shared::Game;
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::auth::AuthBackend;
use crate::game_room::GameRoom;

pub type GameId = Uuid;
pub type GameRooms = Arc<Mutex<HashMap<GameId, Arc<Mutex<GameRoom>>>>>;

#[derive(FromRef, Clone, Debug)]
pub struct AppState {
    pub leptos_options: LeptosOptions,
    pub games: GameRooms,
    pub auth_backend: AuthBackend,
}

impl AppState {
    pub async fn new(leptos_options: LeptosOptions, pool: PgPool) -> Self {
        // GitHub's API rejects requests without a User-Agent header.
        let http_client = reqwest::Client::builder()
            .user_agent(concat!("chess-rs/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build reqwest client");
        AppState {
            leptos_options,
            games: Arc::new(Mutex::new(HashMap::new())),
            auth_backend: AuthBackend::new(pool, http_client).await,
        }
    }

    pub fn create_game(&self, white_player: Uuid, black_player: Uuid) -> GameId {
        let game = GameRoom::new(Game::new(white_player, black_player));
        let game_id = game.game.id;
        let mut games = self.games.blocking_lock();
        games.insert(game_id, Arc::new(Mutex::new(game)));
        game_id
    }

    pub async fn get_game_room(&self, game_id: &GameId) -> Option<Arc<Mutex<GameRoom>>> {
        let games = self.games.lock().await;
        games.get(game_id).cloned()
    }
}
