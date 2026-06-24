use std::{collections::HashMap, sync::Arc};

use axum::extract::FromRef;
use fred::prelude::*;
use futures::channel::mpsc::UnboundedSender;
use leptos::config::LeptosOptions;
use leptos::prelude::ServerFnError;
use shared::{Game, GameConfig, GameStatus, MatchmakingServerMessage, Side};
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::auth::{AuthBackend, AuthError};
use crate::db::{GameStore, RatingStore, UserStore};
use crate::game_room::GameRoom;

pub type GameId = Uuid;
pub type GameRooms = Arc<Mutex<HashMap<GameId, Arc<Mutex<GameRoom>>>>>;
pub type MatchInboxSender = UnboundedSender<Result<MatchmakingServerMessage, ServerFnError>>;
/// Maps user_id → (session_id, sender). The session_id prevents a later tab's
/// cleanup from evicting an earlier tab's — or vice versa — inbox entry.
pub type MatchInbox = Arc<Mutex<HashMap<Uuid, (Uuid, MatchInboxSender)>>>;

#[derive(Debug)]
pub struct PendingMatch {
    pub game_id: GameId,
    pub side: Side,
    created_at: std::time::Instant,
}
pub type PendingMatches = Arc<Mutex<HashMap<Uuid, PendingMatch>>>;

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
    pub pending_matches: PendingMatches,
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
            pending_matches: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[tracing::instrument(skip(self, game_config), fields(white = %white_player, black = %black_player))]
    pub async fn create_game(
        &self,
        game_config: GameConfig,
        white_player: Uuid,
        black_player: Uuid,
        session_score: (f32, f32),
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

    #[tracing::instrument(skip(self, tx), fields(user_id = %id, %session_id))]
    pub async fn add_match_inbox(&self, id: Uuid, session_id: Uuid, tx: MatchInboxSender) {
        let _ = self.match_inboxes.lock().await.insert(id, (session_id, tx));
    }

    /// Remove the inbox entry only if it still belongs to `session_id`.
    /// Guards against a closing tab evicting a later tab's inbox when the same
    /// user has multiple matchmaking connections open simultaneously.
    #[tracing::instrument(skip(self), fields(user_id = %id, %session_id))]
    pub async fn remove_match_inbox(&self, id: &Uuid, session_id: Uuid) {
        let mut map = self.match_inboxes.lock().await;
        if let Some((stored_session, _)) = map.get(id) {
            if *stored_session == session_id {
                map.remove(id);
            }
        }
    }

    #[tracing::instrument(skip(self, message), fields(user_id = %id))]
    pub async fn notify_match(&self, id: Uuid, message: MatchmakingServerMessage) {
        let tx = self.match_inboxes.lock().await.get(&id).map(|(_, tx)| tx.clone());
        if let Some(tx) = tx {
            if tx.unbounded_send(Ok(message)).is_ok() {
                // Message successfully enqueued — consume pending match so a
                // requeue after a fast game doesn't re-navigate to this game.
                self.pending_matches.lock().await.remove(&id);
            }
        }
    }

    pub async fn set_pending_match(&self, player_id: Uuid, game_id: GameId, side: Side) {
        self.pending_matches.lock().await.insert(
            player_id,
            PendingMatch { game_id, side, created_at: std::time::Instant::now() },
        );
    }

    pub async fn take_pending_match(&self, player_id: &Uuid) -> Option<(GameId, Side)> {
        let mut map = self.pending_matches.lock().await;
        match map.remove(player_id) {
            Some(pm) if pm.created_at.elapsed() < std::time::Duration::from_secs(60) => {
                Some((pm.game_id, pm.side))
            }
            _ => None,
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

    /// Add `player_id` to the bucket only if not already present (ZADD NX).
    /// Returns `true` if the entry was newly inserted, `false` if it already
    /// existed. Callers use this to avoid spurious cleanup ZREM calls from
    /// secondary tabs that were blocked by NX.
    #[tracing::instrument(skip(self), fields(%player_id))]
    pub async fn add_to_bucket(
        &self,
        bucket: &str,
        player_id: Uuid,
        rating: u32,
    ) -> FredResult<bool> {
        let added: i64 = self
            .pool
            .zadd(
                bucket,
                Some(SetOptions::NX),
                None,
                false,
                false,
                (rating as f64, player_id.to_string()),
            )
            .await?;
        Ok(added > 0)
    }

    #[tracing::instrument(skip(self), fields(%player_id))]
    pub async fn remove_from_bucket(&self, bucket: &str, player_id: Uuid) -> FredResult<()> {
        self.pool.zrem(bucket, player_id.to_string()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fred::prelude::*;

    async fn make_redis_client() -> RedisClient {
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let config = Config::from_url(&redis_url).expect("invalid REDIS_URL");
        let pool = Pool::new(config, None, None, None, 2).expect("build pool");
        pool.connect();
        pool.wait_for_connect().await.expect("Redis connect");
        RedisClient::new(pool).await
    }

    /// Unique bucket per test so parallel tests don't interfere.
    fn test_bucket(label: &str) -> String {
        format!("test:mm:{}:{}", label, Uuid::new_v4())
    }

    #[tokio::test]
    async fn add_then_remove_from_bucket() {
        let client = make_redis_client().await;
        let bucket = test_bucket("add_remove");
        let player = Uuid::new_v4();

        let was_added = client.add_to_bucket(&bucket, player, 1500).await.unwrap();
        assert!(was_added, "first add should insert");
        let score: Option<f64> = client.pool.zscore(&bucket, player.to_string()).await.unwrap();
        assert_eq!(score, Some(1500.0), "player should be in bucket with correct rating");

        client.remove_from_bucket(&bucket, player).await.unwrap();
        let score_after: Option<f64> = client.pool.zscore(&bucket, player.to_string()).await.unwrap();
        assert!(score_after.is_none(), "player should be removed");
    }

    #[tokio::test]
    async fn add_to_bucket_nx_no_duplicate() {
        let client = make_redis_client().await;
        let bucket = test_bucket("nx");
        let player = Uuid::new_v4();

        let first = client.add_to_bucket(&bucket, player, 1500).await.unwrap();
        assert!(first, "first add returns true");
        // Attempt to add same player with different rating (NX → ignored).
        let second = client.add_to_bucket(&bucket, player, 2000).await.unwrap();
        assert!(!second, "NX-blocked add returns false");
        let score: Option<f64> = client.pool.zscore(&bucket, player.to_string()).await.unwrap();
        assert_eq!(score, Some(1500.0), "NX flag: existing entry must not be overwritten");

        client.remove_from_bucket(&bucket, player).await.unwrap();
    }

    #[tokio::test]
    async fn find_pair_returns_none_when_no_opponent() {
        let client = make_redis_client().await;
        let bucket = test_bucket("no_opp");
        let player = Uuid::new_v4();

        client.add_to_bucket(&bucket, player, 1500).await.unwrap(); // return ignored intentionally
        let result = client.find_pair(&bucket, player, 1500, 100).await.unwrap();
        assert!(result.is_none(), "self-only queue → no match");

        client.remove_from_bucket(&bucket, player).await.unwrap();
    }

    #[tokio::test]
    async fn find_pair_matches_and_removes_both() {
        let client = make_redis_client().await;
        let bucket = test_bucket("match");
        let player_a = Uuid::new_v4();
        let player_b = Uuid::new_v4();

        client.add_to_bucket(&bucket, player_a, 1500).await.unwrap();
        client.add_to_bucket(&bucket, player_b, 1510).await.unwrap();

        let matched = client.find_pair(&bucket, player_a, 1500, 100).await.unwrap();
        assert_eq!(matched, Some(player_b), "should match player_b as nearest in window");

        // Both must be removed atomically.
        let a_score: Option<f64> = client.pool.zscore(&bucket, player_a.to_string()).await.unwrap();
        let b_score: Option<f64> = client.pool.zscore(&bucket, player_b.to_string()).await.unwrap();
        assert!(a_score.is_none(), "player_a should be removed after match");
        assert!(b_score.is_none(), "player_b should be removed after match");
    }

    #[tokio::test]
    async fn find_pair_ignores_out_of_window_opponent() {
        let client = make_redis_client().await;
        let bucket = test_bucket("window");
        let player_a = Uuid::new_v4();
        let player_b = Uuid::new_v4();

        client.add_to_bucket(&bucket, player_a, 1500).await.unwrap();
        client.add_to_bucket(&bucket, player_b, 1700).await.unwrap(); // 200 pts away, window=100

        let result = client.find_pair(&bucket, player_a, 1500, 100).await.unwrap();
        assert!(result.is_none(), "opponent outside rating window must not match");

        client.remove_from_bucket(&bucket, player_a).await.unwrap();
        client.remove_from_bucket(&bucket, player_b).await.unwrap();
    }

    #[tokio::test]
    async fn find_pair_picks_nearest_rating() {
        let client = make_redis_client().await;
        let bucket = test_bucket("nearest");
        let seeker = Uuid::new_v4();
        let close = Uuid::new_v4();
        let far = Uuid::new_v4();

        client.add_to_bucket(&bucket, seeker, 1500).await.unwrap();
        client.add_to_bucket(&bucket, close, 1505).await.unwrap(); // Δ5
        client.add_to_bucket(&bucket, far, 1595).await.unwrap();   // Δ95, still in window

        let matched = client.find_pair(&bucket, seeker, 1500, 100).await.unwrap();
        assert_eq!(matched, Some(close), "should pick nearest rating (close, Δ5)");

        // Clean up the unmatched player.
        client.remove_from_bucket(&bucket, far).await.unwrap();
    }

    #[tokio::test]
    async fn find_pair_returns_none_if_requester_not_in_queue() {
        let client = make_redis_client().await;
        let bucket = test_bucket("not_in_queue");
        let ghost = Uuid::new_v4();
        let opponent = Uuid::new_v4();

        // Ghost is NOT in the bucket; opponent is.
        client.add_to_bucket(&bucket, opponent, 1500).await.unwrap();

        let result = client.find_pair(&bucket, ghost, 1500, 100).await.unwrap();
        assert!(result.is_none(), "requester not in queue → no match");

        client.remove_from_bucket(&bucket, opponent).await.unwrap();
    }
}
