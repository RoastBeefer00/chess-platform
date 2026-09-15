use std::{collections::HashMap, sync::Arc};

use axum::extract::FromRef;
use fred::prelude::*;
use futures::channel::mpsc::UnboundedSender;
use leptos::config::LeptosOptions;
use leptos::prelude::ServerFnError;
use shared::{
    FriendSummary, FriendsServerMessage, Game, GameConfig, GameStatus, MatchmakingServerMessage,
    RatingMode, Side, TimeControl,
};
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::auth::{AuthBackend, AuthError};
use crate::db::{FriendStore, GameStore, PuzzleStore, RatingStore, UserStore};
use crate::game_room::GameRoom;

pub type GameId = Uuid;
pub type GameRooms = Arc<Mutex<HashMap<GameId, Arc<Mutex<GameRoom>>>>>;
pub type MatchInboxSender = UnboundedSender<Result<MatchmakingServerMessage, ServerFnError>>;
/// Maps user_id → (session_id, sender). The session_id prevents a later tab's
/// cleanup from evicting an earlier tab's — or vice versa — inbox entry.
pub type MatchInbox = Arc<Mutex<HashMap<Uuid, (Uuid, MatchInboxSender)>>>;
/// Maps user_id → (tab_count, bucket_key). ZREM only fires when count hits 0
/// so closing one of N tabs never dequeues the player while others are active.
pub type MatchmakingRefcount = Arc<Mutex<HashMap<Uuid, (u32, String)>>>;

#[derive(Debug)]
pub struct PendingMatch {
    pub game_id: GameId,
    pub side: Side,
    created_at: std::time::Instant,
}
pub type PendingMatches = Arc<Mutex<HashMap<Uuid, PendingMatch>>>;

pub type FriendsInboxSender = UnboundedSender<Result<FriendsServerMessage, ServerFnError>>;
/// Maps user_id → session_id → sender. Unlike `MatchInbox`'s single slot per
/// user (where a second tab evicts the first), presence must fan out to
/// every open tab and stay "online" until the *last* one closes — the inner
/// map's `len()` is the tab refcount, so no separate counter is needed.
pub type FriendsInboxes = Arc<Mutex<HashMap<Uuid, HashMap<Uuid, FriendsInboxSender>>>>;

#[derive(Debug, Clone)]
pub struct PendingChallenge {
    pub id: Uuid,
    pub from: Uuid,
    pub from_summary: FriendSummary,
    pub to: Uuid,
    pub time_control: TimeControl,
    pub rating_mode: RatingMode,
    created_at: std::time::Instant,
}

impl PendingChallenge {
    pub fn new(
        from: Uuid,
        from_summary: FriendSummary,
        to: Uuid,
        time_control: TimeControl,
        rating_mode: RatingMode,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            from,
            from_summary,
            to,
            time_control,
            rating_mode,
            created_at: std::time::Instant::now(),
        }
    }
}
/// challenge_id → challenge. In-memory and non-durable on purpose — a 60s
/// offer has no meaning across a restart, and both parties reconnect anyway.
pub type PendingChallenges = Arc<Mutex<HashMap<Uuid, PendingChallenge>>>;
/// Mirrors `PendingMatch`'s 60s window (see `take_pending_match`).
const CHALLENGE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(FromRef, Clone, Debug)]
pub struct AppState {
    pub leptos_options: LeptosOptions,
    pub games: GameRooms,
    pub auth_backend: AuthBackend,
    pub user_store: UserStore,
    pub game_store: GameStore,
    pub puzzle_store: PuzzleStore,
    pub rating_store: RatingStore,
    pub friend_store: FriendStore,
    pub redis_client: RedisClient,
    pub match_inboxes: MatchInbox,
    pub pending_matches: PendingMatches,
    pub matchmaking_refcount: MatchmakingRefcount,
    pub friends_inboxes: FriendsInboxes,
    pub pending_challenges: PendingChallenges,
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
        let puzzle_store = PuzzleStore::new(pool.clone());
        let rating_store = RatingStore::new(pool.clone());
        let friend_store = FriendStore::new(pool.clone());
        let auth_backend = AuthBackend::new(pool, http_client).await;
        AppState {
            leptos_options,
            games: Arc::new(Mutex::new(HashMap::new())),
            auth_backend,
            user_store,
            game_store,
            puzzle_store,
            rating_store,
            friend_store,
            redis_client,
            match_inboxes: Arc::new(Mutex::new(HashMap::new())),
            pending_matches: Arc::new(Mutex::new(HashMap::new())),
            matchmaking_refcount: Arc::new(Mutex::new(HashMap::new())),
            friends_inboxes: Arc::new(Mutex::new(HashMap::new())),
            pending_challenges: Arc::new(Mutex::new(HashMap::new())),
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

        // Best-effort: tell each player's online friends they just started a
        // game, so a friends-list Challenge button disables without needing
        // a page reload. Never pushed on game *end* — that runs from a
        // detached finalize task, not this chokepoint, and staleness there
        // fails safe (button stays disabled a little longer than necessary).
        self.broadcast_in_game(white_player, Some(game_id)).await;
        self.broadcast_in_game(black_player, Some(game_id)).await;

        Ok(game_id)
    }

    /// Notifies `user_id`'s online friends that `user_id` entered (or, if
    /// ever wired up, left) an active game.
    async fn broadcast_in_game(&self, user_id: Uuid, game_id: Option<GameId>) {
        let Ok(friend_ids) = self.friend_store.list_friend_ids(user_id).await else {
            return;
        };
        for friend_id in friend_ids {
            self.notify_friend(friend_id, FriendsServerMessage::InGameUpdate { user_id, game_id })
                .await;
        }
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

    /// Register a matchmaking tab. Returns the new count of active tabs for
    /// this player (1 = first tab, should add to Redis; >1 = already queued).
    pub async fn enter_matchmaking_queue(&self, player_id: Uuid, key: String) -> u32 {
        refcount_enter(&self.matchmaking_refcount, player_id, key).await
    }

    /// Deregister a matchmaking tab. Returns `Some(key)` when this was the
    /// last active tab (caller should ZREM from Redis); `None` otherwise.
    pub async fn leave_matchmaking_queue(&self, player_id: &Uuid) -> Option<String> {
        refcount_leave(&self.matchmaking_refcount, player_id).await
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

    /// Registers a presence tab. Returns `true` when this was the user's
    /// FIRST open tab — the caller should then broadcast
    /// `PresenceUpdate { online: true }` to their friends.
    #[tracing::instrument(skip(self, tx), fields(user_id = %user_id, %session_id))]
    pub async fn add_friends_inbox(&self, user_id: Uuid, session_id: Uuid, tx: FriendsInboxSender) -> bool {
        friends_inbox_add(&self.friends_inboxes, user_id, session_id, tx).await
    }

    /// Deregisters one tab. Returns `true` when this was the LAST tab for
    /// this user (map entry fully removed) — the caller should then
    /// broadcast `PresenceUpdate { online: false }`. Removing an unknown or
    /// already-removed `session_id` is a no-op that returns `false`, so a
    /// stale cleanup can never mark a still-connected user offline.
    #[tracing::instrument(skip(self), fields(user_id = %user_id, %session_id))]
    pub async fn remove_friends_inbox(&self, user_id: &Uuid, session_id: Uuid) -> bool {
        friends_inbox_remove(&self.friends_inboxes, user_id, session_id).await
    }

    /// Fans a message out to every open tab `user_id` has. Prunes senders
    /// whose receiver has already dropped. Returns `true` if at least one
    /// tab received it.
    #[tracing::instrument(skip(self, message), fields(user_id = %user_id))]
    pub async fn notify_friend(&self, user_id: Uuid, message: FriendsServerMessage) -> bool {
        friends_inbox_notify(&self.friends_inboxes, user_id, message).await
    }

    pub async fn is_online(&self, user_id: &Uuid) -> bool {
        self.friends_inboxes.lock().await.contains_key(user_id)
    }

    pub async fn online_among(&self, user_ids: &[Uuid]) -> std::collections::HashSet<Uuid> {
        let map = self.friends_inboxes.lock().await;
        user_ids.iter().copied().filter(|id| map.contains_key(id)).collect()
    }

    pub async fn insert_challenge(&self, challenge: PendingChallenge) {
        self.pending_challenges.lock().await.insert(challenge.id, challenge);
    }

    /// Removes and returns the challenge iff it exists and is within the
    /// TTL. Take-not-peek is what makes a double-accept from two tabs create
    /// exactly one game — the second caller finds nothing.
    pub async fn take_challenge(&self, id: &Uuid) -> Option<PendingChallenge> {
        challenge_take(&self.pending_challenges, id).await
    }

    /// Non-expired challenges addressed to `user_id`, for the reconnect
    /// replay. Sweeps expired entries while holding the lock.
    pub async fn challenges_for(&self, user_id: &Uuid) -> Vec<PendingChallenge> {
        challenges_for_user(&self.pending_challenges, user_id).await
    }

    /// Drops every outstanding challenge sent by `from`, returning them so
    /// the caller can push `ChallengeCancelled` to each target. Call when
    /// the challenger's last tab closes — otherwise a target could accept
    /// into a game against someone who has left the site.
    pub async fn cancel_challenges_from(&self, from: &Uuid) -> Vec<PendingChallenge> {
        challenges_cancel_from(&self.pending_challenges, from).await
    }
}

pub(crate) async fn friends_inbox_add(
    inboxes: &FriendsInboxes,
    user_id: Uuid,
    session_id: Uuid,
    tx: FriendsInboxSender,
) -> bool {
    let mut map = inboxes.lock().await;
    let tabs = map.entry(user_id).or_default();
    let was_empty = tabs.is_empty();
    tabs.insert(session_id, tx);
    was_empty
}

pub(crate) async fn friends_inbox_remove(inboxes: &FriendsInboxes, user_id: &Uuid, session_id: Uuid) -> bool {
    let mut map = inboxes.lock().await;
    let Some(tabs) = map.get_mut(user_id) else { return false };
    if tabs.remove(&session_id).is_none() {
        return false;
    }
    if tabs.is_empty() {
        map.remove(user_id);
        return true;
    }
    false
}

pub(crate) async fn friends_inbox_notify(
    inboxes: &FriendsInboxes,
    user_id: Uuid,
    message: FriendsServerMessage,
) -> bool {
    let mut map = inboxes.lock().await;
    let Some(tabs) = map.get_mut(&user_id) else { return false };
    let mut sent = false;
    tabs.retain(|_, tx| {
        if tx.unbounded_send(Ok(message.clone())).is_ok() {
            sent = true;
            true
        } else {
            false
        }
    });
    if tabs.is_empty() {
        map.remove(&user_id);
    }
    sent
}

pub(crate) async fn challenge_take(challenges: &PendingChallenges, id: &Uuid) -> Option<PendingChallenge> {
    let mut map = challenges.lock().await;
    match map.remove(id) {
        Some(c) if c.created_at.elapsed() < CHALLENGE_TTL => Some(c),
        _ => None,
    }
}

pub(crate) async fn challenges_for_user(challenges: &PendingChallenges, user_id: &Uuid) -> Vec<PendingChallenge> {
    let mut map = challenges.lock().await;
    map.retain(|_, c| c.created_at.elapsed() < CHALLENGE_TTL);
    map.values().filter(|c| c.to == *user_id).cloned().collect()
}

pub(crate) async fn challenges_cancel_from(challenges: &PendingChallenges, from: &Uuid) -> Vec<PendingChallenge> {
    let mut map = challenges.lock().await;
    let ids: Vec<Uuid> = map.values().filter(|c| c.from == *from).map(|c| c.id).collect();
    ids.into_iter().filter_map(|id| map.remove(&id)).collect()
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

pub(crate) async fn refcount_enter(
    refcount: &MatchmakingRefcount,
    player_id: Uuid,
    key: String,
) -> u32 {
    let mut map = refcount.lock().await;
    let entry = map.entry(player_id).or_insert((0, key));
    entry.0 += 1;
    entry.0
}

pub(crate) async fn refcount_leave(
    refcount: &MatchmakingRefcount,
    player_id: &Uuid,
) -> Option<String> {
    let mut map = refcount.lock().await;
    if let Some(entry) = map.get_mut(player_id) {
        entry.0 = entry.0.saturating_sub(1);
        if entry.0 == 0 {
            let key = entry.1.clone();
            map.remove(player_id);
            return Some(key);
        }
    }
    None
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

    fn make_refcount() -> MatchmakingRefcount {
        Arc::new(Mutex::new(HashMap::new()))
    }

    #[tokio::test]
    async fn refcount_first_tab_returns_one() {
        let rc = make_refcount();
        let player = Uuid::new_v4();
        let count = refcount_enter(&rc, player, "bucket".into()).await;
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn refcount_second_tab_returns_two() {
        let rc = make_refcount();
        let player = Uuid::new_v4();
        refcount_enter(&rc, player, "bucket".into()).await;
        let count = refcount_enter(&rc, player, "bucket".into()).await;
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn refcount_leave_non_last_tab_returns_none() {
        let rc = make_refcount();
        let player = Uuid::new_v4();
        refcount_enter(&rc, player, "bucket".into()).await;
        refcount_enter(&rc, player, "bucket".into()).await; // two tabs

        // First tab closes — still one active, no ZREM.
        let result = refcount_leave(&rc, &player).await;
        assert!(result.is_none(), "should not ZREM while another tab is open");
    }

    #[tokio::test]
    async fn refcount_leave_last_tab_returns_key() {
        let rc = make_refcount();
        let player = Uuid::new_v4();
        refcount_enter(&rc, player, "my-bucket".into()).await;
        refcount_enter(&rc, player, "my-bucket".into()).await;

        refcount_leave(&rc, &player).await; // first close → still 1
        let result = refcount_leave(&rc, &player).await; // last close → ZREM
        assert_eq!(result.as_deref(), Some("my-bucket"));
    }

    #[tokio::test]
    async fn refcount_entry_removed_after_last_leave() {
        let rc = make_refcount();
        let player = Uuid::new_v4();
        refcount_enter(&rc, player, "k".into()).await;
        refcount_leave(&rc, &player).await;

        // Map must be empty — no phantom entry with count 0.
        assert!(rc.lock().await.is_empty());
    }

    #[tokio::test]
    async fn refcount_leave_unknown_player_returns_none() {
        let rc = make_refcount();
        let ghost = Uuid::new_v4();
        let result = refcount_leave(&rc, &ghost).await;
        assert!(result.is_none());
    }

    // Regression: models the original bug. Old idle tab (tab_1) closes while
    // tab_2 is still active. Tab_1 should NOT ZREM; tab_2 close should ZREM.
    #[tokio::test]
    async fn refcount_old_tab_close_does_not_zrem_while_new_tab_active() {
        let rc = make_refcount();
        let player = Uuid::new_v4();

        // tab_1 (phone, idle) enters first.
        refcount_enter(&rc, player, "bucket".into()).await;
        // tab_2 (laptop, active) enters.
        refcount_enter(&rc, player, "bucket".into()).await;

        // tab_1 (phone) disconnects via TCP timeout.
        let tab1_result = refcount_leave(&rc, &player).await;
        assert!(tab1_result.is_none(), "tab_1 close must not ZREM — tab_2 still active");

        // tab_2 closes normally.
        let tab2_result = refcount_leave(&rc, &player).await;
        assert_eq!(tab2_result.as_deref(), Some("bucket"), "tab_2 close must ZREM");
    }

    fn make_friends_inboxes() -> FriendsInboxes {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn make_challenges() -> PendingChallenges {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn dummy_sender() -> (FriendsInboxSender, futures::channel::mpsc::UnboundedReceiver<Result<FriendsServerMessage, ServerFnError>>) {
        futures::channel::mpsc::unbounded()
    }

    #[tokio::test]
    async fn friends_inbox_add_first_tab_returns_true() {
        let inboxes = make_friends_inboxes();
        let (tx, _rx) = dummy_sender();
        let first = friends_inbox_add(&inboxes, Uuid::new_v4(), Uuid::new_v4(), tx).await;
        assert!(first);
    }

    #[tokio::test]
    async fn friends_inbox_add_second_tab_returns_false() {
        let inboxes = make_friends_inboxes();
        let user = Uuid::new_v4();
        let (tx1, _rx1) = dummy_sender();
        let (tx2, _rx2) = dummy_sender();
        friends_inbox_add(&inboxes, user, Uuid::new_v4(), tx1).await;
        let second = friends_inbox_add(&inboxes, user, Uuid::new_v4(), tx2).await;
        assert!(!second);
    }

    #[tokio::test]
    async fn friends_inbox_remove_non_last_tab_returns_false() {
        let inboxes = make_friends_inboxes();
        let user = Uuid::new_v4();
        let (tx1, _rx1) = dummy_sender();
        let (tx2, _rx2) = dummy_sender();
        let session1 = Uuid::new_v4();
        friends_inbox_add(&inboxes, user, session1, tx1).await;
        friends_inbox_add(&inboxes, user, Uuid::new_v4(), tx2).await;

        let was_last = friends_inbox_remove(&inboxes, &user, session1).await;
        assert!(!was_last, "closing one of two tabs must not report offline");
        assert!(inboxes.lock().await.contains_key(&user), "user must still be present with one tab left");
    }

    #[tokio::test]
    async fn friends_inbox_remove_last_tab_returns_true_and_clears_entry() {
        let inboxes = make_friends_inboxes();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (tx, _rx) = dummy_sender();
        friends_inbox_add(&inboxes, user, session, tx).await;

        let was_last = friends_inbox_remove(&inboxes, &user, session).await;
        assert!(was_last);
        assert!(!inboxes.lock().await.contains_key(&user), "no phantom empty entry");
    }

    // Regression test for the single-slot MatchInbox bug this design avoids:
    // a stale/already-removed session_id must never flip a still-connected
    // user's presence to offline.
    #[tokio::test]
    async fn friends_inbox_remove_unknown_session_id_is_noop_and_stays_online() {
        let inboxes = make_friends_inboxes();
        let user = Uuid::new_v4();
        let real_session = Uuid::new_v4();
        let stale_session = Uuid::new_v4();
        let (tx, _rx) = dummy_sender();
        friends_inbox_add(&inboxes, user, real_session, tx).await;

        let result = friends_inbox_remove(&inboxes, &user, stale_session).await;
        assert!(!result, "unknown session_id must not report itself as the last tab");
        assert!(inboxes.lock().await.contains_key(&user), "user must remain online — real tab is still open");
    }

    #[tokio::test]
    async fn friends_inbox_remove_unknown_user_returns_false() {
        let inboxes = make_friends_inboxes();
        let result = friends_inbox_remove(&inboxes, &Uuid::new_v4(), Uuid::new_v4()).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn friends_inbox_notify_reaches_all_tabs() {
        let inboxes = make_friends_inboxes();
        let user = Uuid::new_v4();
        let (tx1, mut rx1) = dummy_sender();
        let (tx2, mut rx2) = dummy_sender();
        friends_inbox_add(&inboxes, user, Uuid::new_v4(), tx1).await;
        friends_inbox_add(&inboxes, user, Uuid::new_v4(), tx2).await;

        let sent = friends_inbox_notify(&inboxes, user, FriendsServerMessage::FriendListChanged).await;
        assert!(sent);

        use futures::StreamExt;
        assert!(matches!(rx1.next().await, Some(Ok(FriendsServerMessage::FriendListChanged))));
        assert!(matches!(rx2.next().await, Some(Ok(FriendsServerMessage::FriendListChanged))));
    }

    #[tokio::test]
    async fn friends_inbox_notify_returns_false_for_offline_user() {
        let inboxes = make_friends_inboxes();
        let sent = friends_inbox_notify(&inboxes, Uuid::new_v4(), FriendsServerMessage::FriendListChanged).await;
        assert!(!sent);
    }

    fn make_test_challenge(from: Uuid, to: Uuid) -> PendingChallenge {
        PendingChallenge {
            id: Uuid::new_v4(),
            from,
            from_summary: FriendSummary { id: from, username: None, avatar_url: None },
            to,
            time_control: TimeControl { initial_time: 300_000, mode: shared::TimeMode::Increment(0) },
            rating_mode: RatingMode::Rated,
            created_at: std::time::Instant::now(),
        }
    }

    #[tokio::test]
    async fn take_challenge_returns_it_once_then_none() {
        let challenges = make_challenges();
        let c = make_test_challenge(Uuid::new_v4(), Uuid::new_v4());
        let id = c.id;
        challenges.lock().await.insert(id, c);

        let taken = challenge_take(&challenges, &id).await;
        assert!(taken.is_some());
        let taken_again = challenge_take(&challenges, &id).await;
        assert!(taken_again.is_none(), "double-take must not yield the challenge twice");
    }

    #[tokio::test]
    async fn take_challenge_expired_returns_none() {
        let challenges = make_challenges();
        let mut c = make_test_challenge(Uuid::new_v4(), Uuid::new_v4());
        c.created_at = std::time::Instant::now() - (CHALLENGE_TTL + std::time::Duration::from_secs(1));
        let id = c.id;
        challenges.lock().await.insert(id, c);

        assert!(challenge_take(&challenges, &id).await.is_none());
    }

    #[tokio::test]
    async fn challenges_for_user_filters_by_addressee_and_sweeps_expired() {
        let challenges = make_challenges();
        let target = Uuid::new_v4();
        let live = make_test_challenge(Uuid::new_v4(), target);
        let live_id = live.id;
        let mut expired = make_test_challenge(Uuid::new_v4(), target);
        expired.created_at = std::time::Instant::now() - (CHALLENGE_TTL + std::time::Duration::from_secs(1));
        let expired_id = expired.id;
        let not_for_me = make_test_challenge(Uuid::new_v4(), Uuid::new_v4());

        {
            let mut map = challenges.lock().await;
            map.insert(live_id, live);
            map.insert(expired_id, expired);
            map.insert(not_for_me.id, not_for_me);
        }

        let result = challenges_for_user(&challenges, &target).await;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, live_id);
        assert!(!challenges.lock().await.contains_key(&expired_id), "expired entry must be swept");
    }

    #[tokio::test]
    async fn cancel_challenges_from_removes_only_that_challenger() {
        let challenges = make_challenges();
        let challenger = Uuid::new_v4();
        let mine1 = make_test_challenge(challenger, Uuid::new_v4());
        let mine2 = make_test_challenge(challenger, Uuid::new_v4());
        let others = make_test_challenge(Uuid::new_v4(), Uuid::new_v4());
        let others_id = others.id;

        {
            let mut map = challenges.lock().await;
            map.insert(mine1.id, mine1);
            map.insert(mine2.id, mine2);
            map.insert(others_id, others);
        }

        let cancelled = challenges_cancel_from(&challenges, &challenger).await;
        assert_eq!(cancelled.len(), 2);

        let map = challenges.lock().await;
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(&others_id));
    }
}
