use std::{collections::HashMap, sync::Arc};

use axum::extract::FromRef;
use fred::prelude::*;
use futures::channel::mpsc::UnboundedSender;
use leptos::config::LeptosOptions;
use leptos::prelude::ServerFnError;
use shared::{Game, GameConfig, GameStatus, MatchmakingServerMessage};
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::auth::{AuthBackend, AuthError};
use crate::db::{GameStore, RatingStore, UserStore};
use crate::game_room::GameRoom;

pub type GameId = Uuid;
pub type GameRooms = Arc<Mutex<HashMap<GameId, Arc<Mutex<GameRoom>>>>>;
pub type MatchInboxSender = UnboundedSender<Result<MatchmakingServerMessage, ServerFnError>>;
pub type MatchInbox = Arc<Mutex<HashMap<Uuid, MatchInboxSender>>>;

#[derive(FromRef, Clone, Debug)]
pub struct AppState {
    pub leptos_options: LeptosOptions,
    pub games: GameRooms,
    pub auth_backend: AuthBackend,
    pub user_store: UserStore,
    pub game_store: GameStore,
    pub rating_store: RatingStore,
    pub redis_client: RedisClient,
    pub match_inboxes: MatchInbox,
}

impl AppState {
    pub async fn new(
        leptos_options: LeptosOptions,
        pool: PgPool,
        redis_pool: fred::clients::Pool,
    ) -> Self {
        // GitHub's API rejects requests without a User-Agent header.
        let http_client = reqwest::Client::builder()
            .user_agent(concat!("gambit/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build reqwest client");
        let redis_client = RedisClient::new(redis_pool).await;
        let user_store = UserStore::new(pool.clone());
        let game_store = GameStore::new(pool.clone());
        let rating_store = RatingStore::new(pool.clone());
        let auth_backend = AuthBackend::new(pool, http_client).await;
        AppState {
            leptos_options,
            games: Arc::new(Mutex::new(HashMap::new())),
            auth_backend,
            user_store,
            game_store,
            rating_store,
            redis_client,
            match_inboxes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tracing::instrument(skip(self, game_config), fields(white = %white_player, black = %black_player))]
    pub async fn create_game(
        &self,
        game_config: GameConfig,
        white_player: Uuid,
        black_player: Uuid,
        session_score: (u32, u32),
    ) -> Result<GameId, AuthError> {
        let game = GameRoom::new(Game::new(game_config.clone(), white_player, black_player), session_score);
        let game_id = game.game.id;
        let mut games = self.games.lock().await;
        games.insert(game_id, Arc::new(Mutex::new(game)));
        let initial_time_seconds = (game_config.time_control.initial_time / 1000) as i32;
        let time_increment_seconds = match game_config.time_control.mode {
            shared::TimeMode::Increment(ms) => (ms / 1000) as i32,
            shared::TimeMode::Delay(_) => 0_i32,
        };
        self.game_store
            .insert_new_game(
                &game_id,
                &GameStatus::Ongoing,
                &white_player,
                &black_player,
                &game_config.time_control.category(),
                initial_time_seconds,
                time_increment_seconds,
                game_config.rated.is_rated(),
            )
            .await?;
        tracing::info!(%game_id, "game_created");
        Ok(game_id)
    }

    pub async fn get_game_room(&self, game_id: &GameId) -> Option<Arc<Mutex<GameRoom>>> {
        let games = self.games.lock().await;
        games.get(game_id).cloned()
    }

    #[tracing::instrument(skip(self, tx), fields(user_id = %id))]
    pub async fn add_match_inbox(&self, id: Uuid, tx: MatchInboxSender) {
        let _ = self.match_inboxes.lock().await.insert(id, tx);
    }

    #[tracing::instrument(skip(self), fields(user_id = %id))]
    pub async fn remove_match_inbox(&self, id: &Uuid) {
        let _ = self.match_inboxes.lock().await.remove(id);
    }

    #[tracing::instrument(skip(self, message), fields(user_id = %id))]
    pub async fn notify_match(&self, id: Uuid, message: MatchmakingServerMessage) {
        let tx = self.match_inboxes.lock().await.get(&id).cloned();
        if let Some(tx) = tx {
            let _ = tx.unbounded_send(Ok(message));
        }
    }
}

#[derive(Clone, Debug)]
pub struct RedisClient {
    pool: fred::clients::Pool,
    hash: String,
}

const FIND_PAIR_SCRIPT: &str = include_str!("matchmaking/find_pair.lua");

impl RedisClient {
    pub async fn new(pool: fred::clients::Pool) -> Self {
        let hash = fred::util::sha1_hash(FIND_PAIR_SCRIPT);
        let exists: Vec<bool> = pool
            .script_exists(&hash)
            .await
            .expect("SCRIPT EXISTS on Redis failed at startup");
        if !exists.first().copied().unwrap_or(false) {
            let _: () = pool
                .script_load(FIND_PAIR_SCRIPT)
                .await
                .expect("SCRIPT LOAD on Redis failed at startup");
        }

        Self { pool, hash }
    }

    #[tracing::instrument(skip(self), fields(%player_id))]
    pub async fn find_pair(
        &self,
        bucket: &str,
        player_id: Uuid,
        rating: u32,
        window: u32,
    ) -> FredResult<Option<Uuid>> {
        let opp: Option<String> = self
            .pool
            .evalsha(
                &self.hash,
                vec![bucket],
                vec![
                    player_id.to_string(),
                    rating.to_string(),
                    window.to_string(),
                ],
            )
            .await?;
        Ok(opp.and_then(|s| Uuid::parse_str(&s).ok()))
    }

    #[tracing::instrument(skip(self), fields(%player_id))]
    pub async fn add_to_bucket(
        &self,
        bucket: &str,
        player_id: Uuid,
        rating: u32,
    ) -> FredResult<()> {
        self.pool
            .zadd::<(), _, _>(
                bucket,
                Some(SetOptions::NX),
                None,
                false,
                false,
                (rating as f64, player_id.to_string()),
            )
            .await
    }

    #[tracing::instrument(skip(self), fields(%player_id))]
    pub async fn remove_from_bucket(&self, bucket: &str, player_id: Uuid) -> FredResult<()> {
        self.pool.zrem(bucket, player_id.to_string()).await
    }
}
