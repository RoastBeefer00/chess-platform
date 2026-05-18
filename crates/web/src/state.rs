use std::{collections::HashMap, sync::Arc};

use axum::extract::FromRef;
use fred::prelude::*;
use futures::channel::mpsc::UnboundedSender;
use leptos::config::LeptosOptions;
use leptos::prelude::ServerFnError;
use shared::{Game, MatchmakingServerMessage};
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::auth::AuthBackend;
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
            .user_agent(concat!("chess-rs/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build reqwest client");
        let redis_client = RedisClient::new(redis_pool).await;
        AppState {
            leptos_options,
            games: Arc::new(Mutex::new(HashMap::new())),
            auth_backend: AuthBackend::new(pool, http_client).await,
            redis_client,
            match_inboxes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn create_game(&self, white_player: Uuid, black_player: Uuid) -> GameId {
        let game = GameRoom::new(Game::new(white_player, black_player));
        let game_id = game.game.id;
        let mut games = self.games.lock().await;
        games.insert(game_id, Arc::new(Mutex::new(game)));
        game_id
    }

    pub async fn get_game_room(&self, game_id: &GameId) -> Option<Arc<Mutex<GameRoom>>> {
        let games = self.games.lock().await;
        games.get(game_id).cloned()
    }

    pub async fn add_match_inbox(&self, id: Uuid, tx: MatchInboxSender) {
        let _ = self.match_inboxes.lock().await.insert(id, tx);
    }

    pub async fn remove_match_inbox(&self, id: &Uuid) {
        let _ = self.match_inboxes.lock().await.remove(id);
    }

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
        let exists: Vec<bool> = pool.script_exists(&hash).await.unwrap();
        if !exists.first().copied().unwrap_or(false) {
            let _: () = pool.script_load(FIND_PAIR_SCRIPT).await.unwrap();
        }

        Self { pool, hash }
    }

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

    pub async fn remove_from_bucket(&self, bucket: &str, player_id: Uuid) -> FredResult<()> {
        self.pool.zrem(bucket, player_id.to_string()).await
    }
}
