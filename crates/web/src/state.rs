use std::{collections::HashMap, collections::HashSet, sync::Arc};

use axum::extract::FromRef;
use fred::prelude::*;
use fred::types::{Expiration, Message};
use futures::channel::mpsc::UnboundedSender;
use leptos::config::LeptosOptions;
use leptos::prelude::ServerFnError;
use serde::{Deserialize, Serialize};
use shared::{
    FriendSummary, FriendsServerMessage, Game, GameConfig, GameStatus, MatchmakingServerMessage,
    RatingMode, Side, TimeControl, TimeMode, Variant,
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
/// This map is local to this process: it's the last-mile registry a pub/sub
/// dispatcher (see `RedisClient::new`) delivers into after a cross-instance
/// publish, not the cross-instance source of truth (that's Redis).
pub type MatchInbox = Arc<Mutex<HashMap<Uuid, (Uuid, MatchInboxSender)>>>;
/// Maps user_id → (tab_count, bucket_key). ZREM only fires when count hits 0
/// so closing one of N tabs never dequeues the player while others are active.
/// Deliberately still local/per-instance — see the module doc for why this
/// is a known, low-priority residual gap under N instances.
pub type MatchmakingRefcount = Arc<Mutex<HashMap<Uuid, (u32, String)>>>;

/// A match assignment delivered to `MatchmakingServerMessage::Matched`.
/// Cross-instance state now lives in Redis (`mm:pending:{user_id}`, TTL —
/// see `RedisClient::mm_set_pending_match`/`mm_take_pending_match`); this
/// struct is just the (de)serialization shape, no local map exists anymore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingMatch {
    pub game_id: GameId,
    pub side: Side,
}

pub type FriendsInboxSender = UnboundedSender<Result<FriendsServerMessage, ServerFnError>>;
/// Maps user_id → session_id → sender. Unlike `MatchInbox`'s single slot per
/// user (where a second tab evicts the first), presence must fan out to
/// every open tab and stay "online" until the *last* one closes — the inner
/// map's `len()` is the tab refcount, so no separate counter is needed.
/// Local to this process, same role as `MatchInbox` above: the cross-instance
/// online/offline truth lives in Redis (`friends:online`), this is just
/// where a delivered message gets fanned out to this instance's own sockets.
pub type FriendsInboxes = Arc<Mutex<HashMap<Uuid, HashMap<Uuid, FriendsInboxSender>>>>;

/// A pending friend challenge. Cross-instance state lives in Redis
/// (`challenge:{id}`, TTL — see `RedisClient::challenge_*`); this struct is
/// just the (de)serialization shape. No local map or manual `Instant`-based
/// expiry anymore — Redis's own key TTL is the single source of truth for
/// "has this challenge expired," which is also what makes `challenge_take`
/// atomic and correct regardless of which instance created the challenge or
/// which instance the responder's socket lands on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingChallenge {
    pub id: Uuid,
    pub from: Uuid,
    pub from_summary: FriendSummary,
    pub to: Uuid,
    pub time_control: TimeControl,
    pub rating_mode: RatingMode,
}

impl PendingChallenge {
    pub fn new(
        from: Uuid,
        from_summary: FriendSummary,
        to: Uuid,
        time_control: TimeControl,
        rating_mode: RatingMode,
    ) -> Self {
        Self { id: Uuid::new_v4(), from, from_summary, to, time_control, rating_mode }
    }
}

/// Mirrors the previous in-process 60s window — now the actual Redis key TTL
/// on both `challenge:{id}` and `mm:pending:{user_id}`.
const CHALLENGE_TTL_SECS: i64 = 60;
const PENDING_MATCH_TTL_SECS: i64 = 60;

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
    pub matchmaking_refcount: MatchmakingRefcount,
    pub friends_inboxes: FriendsInboxes,
}

impl AppState {
    pub async fn new(
        leptos_options: LeptosOptions,
        pool: PgPool,
        redis_pool: fred::clients::Pool,
        redis_subscriber: fred::clients::SubscriberClient,
    ) -> Self {
        // GitHub's API rejects requests without a User-Agent header.
        let http_client = reqwest::Client::builder()
            .user_agent(concat!("gambit/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build reqwest client");
        let match_inboxes: MatchInbox = Arc::new(Mutex::new(HashMap::new()));
        let friends_inboxes: FriendsInboxes = Arc::new(Mutex::new(HashMap::new()));
        let redis_client = RedisClient::new(
            redis_pool,
            redis_subscriber,
            friends_inboxes.clone(),
            match_inboxes.clone(),
        )
        .await;
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
            match_inboxes,
            matchmaking_refcount: Arc::new(Mutex::new(HashMap::new())),
            friends_inboxes,
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
        let mut game = GameRoom::new(Game::new(game_config.clone(), white_player, black_player), session_score);
        let game_id = game.game.id;
        let start_fen = {
            use shakmaty::{fen::Fen, EnPassantMode};
            Fen::from_position(&game.get_position(), EnPassantMode::Legal).to_string()
        };

        // Started before insertion so the very first heartbeat can never
        // race a lookup finding the room but not yet ticking. Aborted in
        // `GameRoom::end_game`, same as `timeout_task`/`abort_task`.
        let heartbeat_redis = self.redis_client.clone();
        game.heartbeat_task = Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(ACTIVE_GAME_HEARTBEAT_INTERVAL_SECS)).await;
                heartbeat_redis.refresh_active_game_heartbeat(game_id).await;
            }
        }));

        let mut games = self.games.lock().await;
        games.insert(game_id, Arc::new(Mutex::new(game)));
        drop(games);
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

        // Cross-instance watch-grid roster index + ownership record — see
        // `RedisClient::active_game_upsert`. `instance_id()` is this
        // process's own identity, so any other instance's routing
        // middleware (see `main.rs`) can tell this game is owned here.
        self.redis_client
            .active_game_upsert(
                game_id,
                white_player,
                black_player,
                &game_config.time_control.category().to_string(),
                game_config.rated.is_rated(),
                &start_fen,
                &instance_id(),
            )
            .await;

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

    /// Rebuilds a game from persisted state on an instance that didn't
    /// create it — the "adoption" path for a game whose owning instance
    /// died (crash, autostop, redeploy). Called both from the heartbeat-aware
    /// reaper (`reconcile_stale_active_games`, adopt-before-abort) and from
    /// the websocket join handler when a reconnect finds no local room.
    #[tracing::instrument(skip(self), fields(%game_id))]
    pub async fn adopt_game(&self, game_id: GameId) -> AdoptOutcome {
        use shakmaty::Position as _;

        let row = match self.game_store.load_active_game(game_id).await {
            Ok(Some(row)) => row,
            Ok(None) => return AdoptOutcome::Unadoptable,
            Err(e) => {
                tracing::warn!(?e, %game_id, "adopt_game: failed to load row");
                return AdoptOutcome::Unadoptable;
            }
        };

        let initial_time = i64::from(row.time_initial_seconds) * 1000;
        let config = GameConfig {
            time_control: TimeControl {
                initial_time,
                mode: TimeMode::Increment(i64::from(row.time_increment_seconds) * 1000),
            },
            variant: Variant::Standard,
            rated: if row.rated { RatingMode::Rated } else { RatingMode::Casual },
        };
        let game = Game {
            id: game_id,
            config,
            position: shakmaty::Chess::default(),
            white_player: row.white_user_id,
            black_player: row.black_user_id,
            white_ms_left: initial_time,
            black_ms_left: initial_time,
        };

        let room = match GameRoom::from_persisted(game, row.moves, row.clocks) {
            Ok(room) => room,
            Err(e) => {
                tracing::warn!(%game_id, error = %e, "adopt_game: replay failed, unadoptable");
                return AdoptOutcome::Unadoptable;
            }
        };

        // Check-then-insert must happen under the same guard as the Redis
        // claim below — otherwise two reconnects racing on this instance
        // (or a reaper sweep racing a reconnect) could both pass the
        // `games.get` check before either claims ownership.
        let mut games = self.games.lock().await;
        if let Some(existing) = games.get(&game_id) {
            return AdoptOutcome::Adopted(existing.clone());
        }

        let category = room.game.config.time_control.category().to_string();
        let fen = {
            use shakmaty::{fen::Fen, EnPassantMode};
            Fen::from_position(&room.get_position(), EnPassantMode::Legal).to_string()
        };
        let won = self
            .redis_client
            .claim_active_game(
                game_id,
                row.white_user_id,
                row.black_user_id,
                &category,
                row.rated,
                &fen,
                &instance_id(),
            )
            .await;
        if !won {
            return AdoptOutcome::OwnedElsewhere;
        }

        let turn = room.game.position.turn();
        let ms_left = match turn {
            shakmaty::Color::White => room.game.white_ms_left,
            shakmaty::Color::Black => room.game.black_ms_left,
        };

        let heartbeat_redis = self.redis_client.clone();
        let mut room = room;
        room.heartbeat_task = Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(ACTIVE_GAME_HEARTBEAT_INTERVAL_SECS)).await;
                heartbeat_redis.refresh_active_game_heartbeat(game_id).await;
            }
        }));

        let room_arc = Arc::new(Mutex::new(room));
        games.insert(game_id, room_arc.clone());
        drop(games);
        tracing::info!(%game_id, "game_adopted");

        // Re-arm the flag-fall timer. Safe even for a zero-move (still
        // `WaitingForOpponent`) room — `handle_timeout` bails immediately
        // if the game isn't `Ongoing`, and the next real move replaces this
        // task anyway (see the move path in `websocket.rs`).
        let timeout_handle = tokio::spawn(crate::game_room::handle_timeout(
            room_arc.clone(),
            self.game_store.clone(),
            self.redis_client.clone(),
            turn,
            ms_left,
        ));
        room_arc.lock().await.timeout_task = Some(timeout_handle);

        AdoptOutcome::Adopted(room_arc)
    }

    /// Removes a finished/aborted game's entry from the cross-instance
    /// watch-grid index. Safe to call even if the entry never existed (a
    /// no-op) — every finalize/abort path calls this so a game never
    /// lingers in the roster past its own end.
    pub async fn deactivate_game(&self, game_id: GameId) {
        self.redis_client.active_game_remove(game_id).await;
    }

    /// The heartbeat-aware reaper. Every DB row still `status='active'` is
    /// checked against `active_games:{id}` in Redis (see
    /// `RedisClient::active_game_owner`, `GameRoom::heartbeat_task`): if the
    /// entry is still there, some instance — this one or a healthy peer —
    /// still owns it, so it's left alone. If it's gone, that game's owning
    /// instance has genuinely gone quiet (crashed, restarted, autostopped) —
    /// but rather than assuming that means the game is lost, this now tries
    /// to **adopt** it first (see `adopt_game`): moves/clocks already
    /// persist incrementally, so a game killed mid-flight has everything
    /// needed to pick back up. Only a game that genuinely can't be adopted
    /// (unreplayable move data, or a peer wins the claim first) still gets
    /// aborted here — this is the safety net, not the common case anymore.
    /// Run both at boot and periodically (see `main.rs`), since a peer can
    /// go quiet at any time, not just when this instance happens to be
    /// starting.
    #[tracing::instrument(skip(self))]
    pub async fn reconcile_stale_active_games(&self) -> Result<u64, AuthError> {
        let active_ids = self.game_store.list_active_game_ids().await?;
        if active_ids.is_empty() {
            return Ok(0);
        }

        let mut stale = Vec::new();
        for id in active_ids {
            if self.redis_client.active_game_owner(id).await.is_none() {
                stale.push(id);
            }
        }
        if stale.is_empty() {
            return Ok(0);
        }

        let mut unadoptable = Vec::new();
        for id in stale {
            // A zero-move orphan has no history worth preserving and would
            // otherwise sit `WaitingForOpponent` with a live heartbeat
            // forever — let it abort as it always has. A reconnect can
            // still adopt it directly (see `websocket.rs`), where a player
            // being present makes the room worth keeping.
            let moves_empty = self
                .game_store
                .load_active_game(id)
                .await
                .ok()
                .flatten()
                .map(|row| row.moves.is_empty())
                .unwrap_or(true);
            if moves_empty {
                unadoptable.push(id);
                continue;
            }
            match self.adopt_game(id).await {
                AdoptOutcome::Adopted(_) => {
                    tracing::info!(game_id = %id, "reconciled_stale_active_game: adopted");
                }
                AdoptOutcome::OwnedElsewhere => {
                    tracing::info!(game_id = %id, "reconciled_stale_active_game: claimed by a peer");
                }
                AdoptOutcome::Unadoptable => unadoptable.push(id),
            }
        }
        if unadoptable.is_empty() {
            return Ok(0);
        }

        let count = self.game_store.abort_stale_games(&unadoptable).await?;
        if count > 0 {
            tracing::warn!(count, ids = ?unadoptable, "reconciled_stale_active_games: aborted unadoptable");
        }
        Ok(count)
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

    /// Publishes to Redis so whichever instance holds `id`'s live
    /// matchmaking socket (this one or another) delivers it — see the
    /// pattern-subscription dispatcher started in `RedisClient::new`. Also
    /// clears any pending-match fallback, mirroring the old local-map
    /// behavior (a delivered push means the reconnect fallback is moot).
    #[tracing::instrument(skip(self, message), fields(user_id = %id))]
    pub async fn notify_match(&self, id: Uuid, message: MatchmakingServerMessage) {
        self.redis_client.mm_publish(id, &message).await;
        self.redis_client.mm_take_pending_match(id).await;
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
        self.redis_client.mm_set_pending_match(player_id, PendingMatch { game_id, side }).await;
    }

    pub async fn take_pending_match(&self, player_id: &Uuid) -> Option<(GameId, Side)> {
        self.redis_client.mm_take_pending_match(*player_id).await.map(|pm| (pm.game_id, pm.side))
    }

    /// Registers a presence tab. Returns `true` when this was the user's
    /// FIRST open tab across ALL instances — the caller should then
    /// broadcast `PresenceUpdate { online: true }` to their friends.
    #[tracing::instrument(skip(self, tx), fields(user_id = %user_id, %session_id))]
    pub async fn add_friends_inbox(&self, user_id: Uuid, session_id: Uuid, tx: FriendsInboxSender) -> bool {
        // Local bookkeeping first: this is what the pub/sub dispatcher reads
        // to decide whether THIS instance should deliver an incoming
        // message locally. The Redis call right after decides the globally
        // correct first/last-tab transition for the online set.
        friends_inbox_add(&self.friends_inboxes, user_id, session_id, tx).await;
        let session = friends_session_key(session_id);
        self.redis_client.friends_mark_online(user_id, &session).await
    }

    /// Deregisters one tab. Returns `true` when this was the LAST tab for
    /// this user ACROSS ALL INSTANCES — the caller should then broadcast
    /// `PresenceUpdate { online: false }`. Removing an unknown or
    /// already-removed `session_id` is a no-op that returns `false`, so a
    /// stale cleanup can never mark a still-connected user offline.
    #[tracing::instrument(skip(self), fields(user_id = %user_id, %session_id))]
    pub async fn remove_friends_inbox(&self, user_id: &Uuid, session_id: Uuid) -> bool {
        let removed_locally = friends_inbox_remove(&self.friends_inboxes, user_id, session_id).await;
        if !removed_locally {
            return false;
        }
        let session = friends_session_key(session_id);
        self.redis_client.friends_mark_offline(*user_id, &session).await
    }

    /// Publishes to Redis so every instance holding a live tab for `user_id`
    /// delivers it locally — see the pattern-subscription dispatcher started
    /// in `RedisClient::new`.
    #[tracing::instrument(skip(self, message), fields(user_id = %user_id))]
    pub async fn notify_friend(&self, user_id: Uuid, message: FriendsServerMessage) {
        self.redis_client.friends_publish(user_id, &message).await;
    }

    pub async fn is_online(&self, user_id: &Uuid) -> bool {
        self.redis_client.friends_is_online(*user_id).await
    }

    pub async fn online_among(&self, user_ids: &[Uuid]) -> HashSet<Uuid> {
        self.redis_client.friends_online_among(user_ids).await
    }

    pub async fn insert_challenge(&self, challenge: PendingChallenge) {
        self.redis_client.challenge_insert(&challenge).await;
    }

    /// Removes and returns the challenge iff it exists and is within the
    /// TTL (enforced natively by Redis key expiry). Take-not-peek is what
    /// makes a double-accept from two tabs create exactly one game — the
    /// second caller finds nothing, regardless of which instance it lands on.
    pub async fn take_challenge(&self, id: &Uuid) -> Option<PendingChallenge> {
        self.redis_client.challenge_take(*id).await
    }

    /// Non-expired challenges addressed to `user_id`, for the reconnect
    /// replay.
    pub async fn challenges_for(&self, user_id: &Uuid) -> Vec<PendingChallenge> {
        self.redis_client.challenges_for_user(*user_id).await
    }

    /// Drops every outstanding challenge sent by `from`, returning them so
    /// the caller can push `ChallengeCancelled` to each target. Call when
    /// the challenger's last tab closes — otherwise a target could accept
    /// into a game against someone who has left the site.
    pub async fn cancel_challenges_from(&self, from: &Uuid) -> Vec<PendingChallenge> {
        self.redis_client.challenges_cancel_from(*from).await
    }
}

/// This process's own identity — `FLY_MACHINE_ID` in production (injected
/// automatically by Fly Machines), a fixed fallback in local dev where
/// there's only ever one instance anyway.
pub(crate) fn instance_id() -> String {
    std::env::var("FLY_MACHINE_ID").unwrap_or_else(|_| "local".to_string())
}

fn friends_session_key(session_id: Uuid) -> String {
    // Instance-qualified so two instances issuing the same random session_id
    // (astronomically unlikely, but free to guard against) can't collide in
    // the shared Redis sessions set.
    format!("{}:{session_id}", instance_id())
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

/// Delivers to every LOCAL tab this instance holds for `user_id`. This is
/// the last-mile half of friend notification — the pub/sub dispatcher in
/// `RedisClient::new` calls this after receiving a published message; it's
/// no longer called directly by `AppState::notify_friend` (see there).
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

/// Delivers to the LOCAL matchmaking inbox for `user_id`, if this instance
/// holds one. Mirrors `friends_inbox_notify`'s role for the friends map.
pub(crate) async fn match_inbox_notify(inboxes: &MatchInbox, user_id: Uuid, message: MatchmakingServerMessage) {
    let tx = inboxes.lock().await.get(&user_id).map(|(_, tx)| tx.clone());
    if let Some(tx) = tx {
        let _ = tx.unbounded_send(Ok(message));
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

/// Routes an incoming pub/sub message to the right local inbox map based on
/// its channel prefix. One subscriber client (see `RedisClient::new`)
/// pattern-subscribes to both `friends:user:*` and `mm:user:*`, so every
/// instance's dispatcher sees every published message regardless of which
/// instance published it — cheap at this app's scale, and far simpler than
/// per-user dynamic subscribe/unsubscribe management.
async fn dispatch_pubsub_message(friends_inboxes: &FriendsInboxes, match_inboxes: &MatchInbox, message: Message) {
    let Some(payload) = message.value.as_str() else { return };
    if let Some(user_id_str) = message.channel.strip_prefix("friends:user:") {
        let Ok(user_id) = Uuid::parse_str(user_id_str) else { return };
        let Ok(parsed) = serde_json::from_str::<FriendsServerMessage>(&payload) else { return };
        friends_inbox_notify(friends_inboxes, user_id, parsed).await;
    } else if let Some(user_id_str) = message.channel.strip_prefix("mm:user:") {
        let Ok(user_id) = Uuid::parse_str(user_id_str) else { return };
        let Ok(parsed) = serde_json::from_str::<MatchmakingServerMessage>(&payload) else { return };
        match_inbox_notify(match_inboxes, user_id, parsed).await;
    }
}

#[derive(Clone, Debug)]
pub struct RedisClient {
    pool: fred::clients::Pool,
    find_pair_hash: String,
    presence_hash: String,
    claim_game_hash: String,
}

const FIND_PAIR_SCRIPT: &str = include_str!("matchmaking/find_pair.lua");
const PRESENCE_SCRIPT: &str = include_str!("friends/presence.lua");
const CLAIM_GAME_SCRIPT: &str = include_str!("claim_game.lua");

/// TTL on `active_games:{id}` — the heartbeat for game ownership. Renewed
/// every `ACTIVE_GAME_HEARTBEAT_INTERVAL_SECS` by `GameRoom::heartbeat_task`
/// and on every move; a healthy owning instance can never let this lapse.
/// 3x the renewal interval mirrors the same check:timeout ratio idiom
/// already used for the client-side game-socket heartbeat
/// (`HEARTBEAT_CHECK_MS`/`HEARTBEAT_TIMEOUT_MS` in `play_board/ws_session.rs`).
const ACTIVE_GAME_TTL_SECS: i64 = 30;
pub const ACTIVE_GAME_HEARTBEAT_INTERVAL_SECS: u64 = 10;

impl RedisClient {
    pub async fn new(
        pool: fred::clients::Pool,
        subscriber: fred::clients::SubscriberClient,
        friends_inboxes: FriendsInboxes,
        match_inboxes: MatchInbox,
    ) -> Self {
        let find_pair_hash = Self::load_script(&pool, FIND_PAIR_SCRIPT).await;
        let presence_hash = Self::load_script(&pool, PRESENCE_SCRIPT).await;
        let claim_game_hash = Self::load_script(&pool, CLAIM_GAME_SCRIPT).await;

        subscriber.connect();
        subscriber
            .wait_for_connect()
            .await
            .expect("Redis subscriber failed initial connect");
        // Auto-resubscribe on reconnect — otherwise a dropped subscriber
        // connection would silently stop delivering cross-instance friend
        // and matchmaking messages until the next deploy.
        subscriber.manage_subscriptions();
        subscriber
            .psubscribe(vec!["friends:user:*", "mm:user:*"])
            .await
            .expect("Redis PSUBSCRIBE failed at startup");
        subscriber.on_message(move |message: Message| {
            let friends_inboxes = friends_inboxes.clone();
            let match_inboxes = match_inboxes.clone();
            async move {
                dispatch_pubsub_message(&friends_inboxes, &match_inboxes, message).await;
                Ok(())
            }
        });

        Self { pool, find_pair_hash, presence_hash, claim_game_hash }
    }

    async fn load_script(pool: &fred::clients::Pool, script: &str) -> String {
        let hash = fred::util::sha1_hash(script);
        let exists: Vec<bool> = pool
            .script_exists(&hash)
            .await
            .expect("SCRIPT EXISTS on Redis failed at startup");
        if !exists.first().copied().unwrap_or(false) {
            let _: () = pool
                .script_load(script)
                .await
                .expect("SCRIPT LOAD on Redis failed at startup");
        }
        hash
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
                &self.find_pair_hash,
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

    // --- Friends presence ---

    async fn friends_presence_transition(&self, action: &str, user_id: Uuid, session: &str) -> bool {
        let sessions_key = format!("friends:sessions:{user_id}");
        let result: i64 = self
            .pool
            .evalsha(
                &self.presence_hash,
                vec!["friends:online", sessions_key.as_str()],
                vec![action.to_string(), session.to_string(), user_id.to_string()],
            )
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(?e, %user_id, "friends presence transition failed");
                0
            });
        result == 1
    }

    /// Returns `true` iff this was `user_id`'s first tab across all instances.
    pub async fn friends_mark_online(&self, user_id: Uuid, session: &str) -> bool {
        self.friends_presence_transition("add", user_id, session).await
    }

    /// Returns `true` iff this was `user_id`'s last tab across all instances.
    pub async fn friends_mark_offline(&self, user_id: Uuid, session: &str) -> bool {
        self.friends_presence_transition("remove", user_id, session).await
    }

    pub async fn friends_is_online(&self, user_id: Uuid) -> bool {
        self.pool.sismember("friends:online", user_id.to_string()).await.unwrap_or(false)
    }

    /// Fetches the whole online set once and intersects locally rather than
    /// N round trips — fine at this app's scale (a user's friend list and
    /// the online set are both small).
    pub async fn friends_online_among(&self, user_ids: &[Uuid]) -> HashSet<Uuid> {
        if user_ids.is_empty() {
            return HashSet::new();
        }
        let members: Vec<String> = self.pool.smembers("friends:online").await.unwrap_or_default();
        let online: HashSet<Uuid> = members.iter().filter_map(|s| Uuid::parse_str(s).ok()).collect();
        user_ids.iter().copied().filter(|id| online.contains(id)).collect()
    }

    /// Publishes to `friends:user:{user_id}` — delivered to every instance's
    /// dispatcher (see `RedisClient::new`), which fans out to that
    /// instance's own local tabs for this user, if any. Best-effort: a
    /// failure here is logged, never surfaced to the caller, matching the
    /// old local-map behavior where every call site already ignored the
    /// return value.
    pub async fn friends_publish(&self, user_id: Uuid, message: &FriendsServerMessage) {
        let payload: String = match serde_json::to_string(message) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(?e, "failed to serialize FriendsServerMessage");
                return;
            }
        };
        let result: FredResult<i64> =
            self.pool.next().publish(format!("friends:user:{user_id}"), payload).await;
        if let Err(e) = result {
            tracing::warn!(?e, %user_id, "friends_publish failed");
        }
    }

    // --- Matchmaking push ---

    pub async fn mm_publish(&self, user_id: Uuid, message: &MatchmakingServerMessage) {
        let payload: String = match serde_json::to_string(message) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(?e, "failed to serialize MatchmakingServerMessage");
                return;
            }
        };
        let result: FredResult<i64> =
            self.pool.next().publish(format!("mm:user:{user_id}"), payload).await;
        if let Err(e) = result {
            tracing::warn!(?e, %user_id, "mm_publish failed");
        }
    }

    pub async fn mm_set_pending_match(&self, user_id: Uuid, pending: PendingMatch) {
        let Ok(payload) = serde_json::to_string(&pending) else { return };
        if let Err(e) = self
            .pool
            .set::<(), _, _>(
                format!("mm:pending:{user_id}"),
                payload,
                Some(Expiration::EX(PENDING_MATCH_TTL_SECS)),
                None,
                false,
            )
            .await
        {
            tracing::warn!(?e, %user_id, "mm_set_pending_match failed");
        }
    }

    /// Atomic get-and-delete via Redis `GETDEL` — the TTL set in
    /// `mm_set_pending_match` is the entire expiry mechanism, no manual
    /// `Instant` bookkeeping needed.
    pub async fn mm_take_pending_match(&self, user_id: Uuid) -> Option<PendingMatch> {
        let raw: Option<String> = self.pool.getdel(format!("mm:pending:{user_id}")).await.ok().flatten();
        raw.and_then(|s| serde_json::from_str(&s).ok())
    }

    // --- Challenges ---

    pub async fn challenge_insert(&self, challenge: &PendingChallenge) {
        let Ok(payload) = serde_json::to_string(challenge) else { return };
        let key = format!("challenge:{}", challenge.id);
        if let Err(e) = self
            .pool
            .set::<(), _, _>(&key, payload, Some(Expiration::EX(CHALLENGE_TTL_SECS)), None, false)
            .await
        {
            tracing::warn!(?e, challenge_id = %challenge.id, "challenge_insert failed");
            return;
        }
        // Index sets for `challenges_for_user`/`challenges_cancel_from`.
        // Members here can point at an already-expired/taken challenge —
        // both readers double-check the challenge key itself still exists,
        // so a stale index entry just gets silently filtered, never
        // resurrects an answered challenge.
        let to_key = format!("challenges:to:{}", challenge.to);
        let from_key = format!("challenges:from:{}", challenge.from);
        let id = challenge.id.to_string();
        let _: Result<(), _> = self.pool.sadd(&to_key, id.as_str()).await;
        let _: Result<(), _> = self.pool.expire::<(), _>(&to_key, CHALLENGE_TTL_SECS, None).await;
        let _: Result<(), _> = self.pool.sadd(&from_key, id.as_str()).await;
        let _: Result<(), _> = self.pool.expire::<(), _>(&from_key, CHALLENGE_TTL_SECS, None).await;
    }

    /// Atomic get-and-delete — makes a double-accept from two tabs/instances
    /// yield the challenge to exactly one caller.
    pub async fn challenge_take(&self, id: Uuid) -> Option<PendingChallenge> {
        let raw: Option<String> = self.pool.getdel(format!("challenge:{id}")).await.ok().flatten();
        raw.and_then(|s| serde_json::from_str(&s).ok())
    }

    pub async fn challenges_for_user(&self, user_id: Uuid) -> Vec<PendingChallenge> {
        let ids: Vec<String> = self
            .pool
            .smembers(format!("challenges:to:{user_id}"))
            .await
            .unwrap_or_default();
        let mut result = Vec::new();
        for id in ids {
            let raw: Option<String> = self.pool.get(format!("challenge:{id}")).await.ok().flatten();
            if let Some(c) = raw.and_then(|s| serde_json::from_str::<PendingChallenge>(&s).ok()) {
                result.push(c);
            }
        }
        result
    }

    pub async fn challenges_cancel_from(&self, from: Uuid) -> Vec<PendingChallenge> {
        let ids: Vec<String> = self
            .pool
            .smembers(format!("challenges:from:{from}"))
            .await
            .unwrap_or_default();
        let mut result = Vec::new();
        for id in ids {
            let raw: Option<String> = self.pool.getdel(format!("challenge:{id}")).await.ok().flatten();
            if let Some(c) = raw.and_then(|s| serde_json::from_str::<PendingChallenge>(&s).ok()) {
                result.push(c);
            }
        }
        let _: Result<(), _> = self.pool.del(format!("challenges:from:{from}")).await;
        result
    }

    // --- Cross-instance watch-grid roster index ---
    //
    // `active_games:{game_id}` is a small hash any instance can read to
    // render a game it doesn't locally own in the watch grid, AND the
    // ownership/heartbeat record a routing middleware and (later) a reaper
    // rely on. Written on creation and on every move (see the move path in
    // `websocket.rs`), removed on finalize/abort/timeout/resign/draw. Its
    // TTL (`ACTIVE_GAME_TTL_SECS`) is the heartbeat: refreshed on every move
    // and by a periodic per-room task (`GameRoom::heartbeat_task`) so a
    // slow-clock game with long gaps between moves doesn't look stale. A
    // reaper checking this hash's mere existence (not a manual timestamp
    // comparison) is what lets it tell "orphaned" (owning instance gone,
    // heartbeat lapsed, key expired) from "owned by a healthy peer" (key
    // still there).

    #[allow(clippy::too_many_arguments)]
    pub async fn active_game_upsert(
        &self,
        game_id: Uuid,
        white_id: Uuid,
        black_id: Uuid,
        category: &str,
        rated: bool,
        fen: &str,
        owner_instance: &str,
    ) {
        let key = format!("active_games:{game_id}");
        let fields: Vec<(&str, String)> = vec![
            ("white_id", white_id.to_string()),
            ("black_id", black_id.to_string()),
            ("category", category.to_string()),
            ("rated", if rated { "1" } else { "0" }.to_string()),
            ("fen", fen.to_string()),
            ("owner_instance", owner_instance.to_string()),
        ];
        if let Err(e) = self.pool.hset::<(), _, _>(&key, fields).await {
            tracing::warn!(?e, %game_id, "active_game_upsert failed");
            return;
        }
        if let Err(e) = self.pool.expire::<(), _>(&key, ACTIVE_GAME_TTL_SECS, None).await {
            tracing::warn!(?e, %game_id, "active_game_upsert: setting TTL failed");
        }
    }

    /// Atomic compare-and-set claim on an *ownerless* `active_games:{id}`
    /// entry — the adoption counterpart to `active_game_upsert` (which is a
    /// plain unconditional `HSET`, correct only for a genuinely new game).
    /// Two instances racing to adopt the same orphan must not both "win," or
    /// the two players end up on split-brain rooms on different instances —
    /// see `claim_game.lua`. Returns `true` if this call created the record
    /// (claim won), `false` if it already existed (a peer beat us to it, or
    /// the game was never actually ownerless).
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(self), fields(%game_id))]
    pub async fn claim_active_game(
        &self,
        game_id: Uuid,
        white_id: Uuid,
        black_id: Uuid,
        category: &str,
        rated: bool,
        fen: &str,
        owner_instance: &str,
    ) -> bool {
        let key = format!("active_games:{game_id}");
        let won: i64 = self
            .pool
            .evalsha(
                &self.claim_game_hash,
                vec![key],
                vec![
                    white_id.to_string(),
                    black_id.to_string(),
                    category.to_string(),
                    if rated { "1" } else { "0" }.to_string(),
                    fen.to_string(),
                    owner_instance.to_string(),
                    ACTIVE_GAME_TTL_SECS.to_string(),
                ],
            )
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(?e, %game_id, "claim_active_game failed");
                0
            });
        won == 1
    }

    /// Cheaper partial update for the per-move case — only `fen` changes.
    /// Also refreshes the TTL, so active play is itself a heartbeat signal
    /// independent of the periodic `heartbeat_task` tick.
    pub async fn active_game_update_fen(&self, game_id: Uuid, fen: &str) {
        let key = format!("active_games:{game_id}");
        if let Err(e) = self.pool.hset::<(), _, _>(&key, vec![("fen", fen.to_string())]).await {
            tracing::warn!(?e, %game_id, "active_game_update_fen failed");
            return;
        }
        if let Err(e) = self.pool.expire::<(), _>(&key, ACTIVE_GAME_TTL_SECS, None).await {
            tracing::warn!(?e, %game_id, "active_game_update_fen: refreshing TTL failed");
        }
    }

    /// The periodic heartbeat tick — see `GameRoom::heartbeat_task`. A no-op
    /// on a key that's already gone (game already ended and was removed),
    /// consistent with every other method here treating a missing entry as
    /// "nothing to do" rather than an error.
    pub async fn refresh_active_game_heartbeat(&self, game_id: Uuid) {
        let _: Result<(), _> = self
            .pool
            .expire::<(), _>(format!("active_games:{game_id}"), ACTIVE_GAME_TTL_SECS, None)
            .await;
    }

    /// The instance id that owns `game_id`, if the entry exists (and hasn't
    /// expired). `None` means either the game never existed here or its
    /// heartbeat has lapsed — both cases a caller should treat as "not
    /// reliably routable to a specific instance right now."
    pub async fn active_game_owner(&self, game_id: Uuid) -> Option<String> {
        self.pool.hget(format!("active_games:{game_id}"), "owner_instance").await.ok().flatten()
    }

    pub async fn active_game_remove(&self, game_id: Uuid) {
        let _: Result<(), _> = self.pool.del(format!("active_games:{game_id}")).await;
    }

    /// All currently-indexed active games except the given ids (already
    /// known locally to the caller, e.g. this instance's own `GameRooms`).
    /// Uses `SCAN` rather than `KEYS` to avoid blocking Redis on a large
    /// keyspace — acceptable at this app's game-count scale either way, but
    /// SCAN costs nothing extra and is the safer default.
    pub async fn active_games_excluding(&self, exclude: &HashSet<Uuid>) -> Vec<ActiveGameEntry> {
        let mut cursor = "0".to_string();
        let mut ids = Vec::new();
        loop {
            let (next_cursor, keys): (String, Vec<String>) = match self
                .pool
                .scan_page(cursor, "active_games:*", Some(100), None)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(?e, "active_games scan failed");
                    break;
                }
            };
            ids.extend(keys);
            if next_cursor == "0" {
                break;
            }
            cursor = next_cursor;
        }

        let mut result = Vec::new();
        for key in ids {
            let Some(game_id_str) = key.strip_prefix("active_games:") else { continue };
            let Ok(game_id) = Uuid::parse_str(game_id_str) else { continue };
            if exclude.contains(&game_id) {
                continue;
            }
            let fields: HashMap<String, String> = self.pool.hgetall(&key).await.unwrap_or_default();
            let (Some(white_id), Some(black_id), Some(category), Some(rated), Some(fen)) = (
                fields.get("white_id").and_then(|s| Uuid::parse_str(s).ok()),
                fields.get("black_id").and_then(|s| Uuid::parse_str(s).ok()),
                fields.get("category").cloned(),
                fields.get("rated").map(|s| s == "1"),
                fields.get("fen").cloned(),
            ) else {
                continue;
            };
            result.push(ActiveGameEntry { game_id, white_id, black_id, category, rated, fen });
        }
        result
    }
}

/// Result of `AppState::adopt_game`.
pub enum AdoptOutcome {
    /// Rebuilt and registered locally (or already was, by a racing task on
    /// this same instance) — the caller can treat this exactly like
    /// `get_game_room` returning `Some`.
    Adopted(Arc<Mutex<GameRoom>>),
    /// A peer instance claimed it first. Not an error — the caller should
    /// fail this attempt so the client reconnects and gets routed to the
    /// real owner.
    OwnedElsewhere,
    /// No row, or its move history doesn't replay (corrupt data, or a
    /// variant/start position adoption can't reconstruct). The caller
    /// should fall back to today's abort/"not found" handling.
    Unadoptable,
}

/// One cross-instance-visible active game, as read back from Redis for the
/// watch-grid roster.
#[derive(Debug, Clone)]
pub struct ActiveGameEntry {
    pub game_id: Uuid,
    pub white_id: Uuid,
    pub black_id: Uuid,
    pub category: String,
    pub rated: bool,
    pub fen: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use fred::prelude::*;

    async fn make_redis_client_with_inboxes() -> (RedisClient, FriendsInboxes, MatchInbox) {
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let config = Config::from_url(&redis_url).expect("invalid REDIS_URL");
        let pool = Pool::new(config.clone(), None, None, None, 2).expect("build pool");
        pool.connect();
        pool.wait_for_connect().await.expect("Redis connect");
        let subscriber = fred::clients::SubscriberClient::new(config, None, None, None);
        let friends_inboxes: FriendsInboxes = Arc::new(Mutex::new(HashMap::new()));
        let match_inboxes: MatchInbox = Arc::new(Mutex::new(HashMap::new()));
        let client = RedisClient::new(pool, subscriber, friends_inboxes.clone(), match_inboxes.clone()).await;
        (client, friends_inboxes, match_inboxes)
    }

    async fn make_redis_client() -> RedisClient {
        make_redis_client_with_inboxes().await.0
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
        }
    }

    #[tokio::test]
    async fn challenge_take_returns_it_once_then_none() {
        let client = make_redis_client().await;
        let c = make_test_challenge(Uuid::new_v4(), Uuid::new_v4());
        let id = c.id;
        client.challenge_insert(&c).await;

        let taken = client.challenge_take(id).await;
        assert!(taken.is_some());
        let taken_again = client.challenge_take(id).await;
        assert!(taken_again.is_none(), "double-take must not yield the challenge twice");
    }

    #[tokio::test]
    async fn challenges_for_user_filters_by_addressee() {
        let client = make_redis_client().await;
        let target = Uuid::new_v4();
        let live = make_test_challenge(Uuid::new_v4(), target);
        let live_id = live.id;
        let not_for_me = make_test_challenge(Uuid::new_v4(), Uuid::new_v4());
        client.challenge_insert(&live).await;
        client.challenge_insert(&not_for_me).await;

        let result = client.challenges_for_user(target).await;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, live_id);
    }

    #[tokio::test]
    async fn cancel_challenges_from_removes_only_that_challenger() {
        let client = make_redis_client().await;
        let challenger = Uuid::new_v4();
        let mine1 = make_test_challenge(challenger, Uuid::new_v4());
        let mine2 = make_test_challenge(challenger, Uuid::new_v4());
        let others = make_test_challenge(Uuid::new_v4(), Uuid::new_v4());
        let others_id = others.id;
        client.challenge_insert(&mine1).await;
        client.challenge_insert(&mine2).await;
        client.challenge_insert(&others).await;

        let cancelled = client.challenges_cancel_from(challenger).await;
        assert_eq!(cancelled.len(), 2);

        // The other challenger's own challenge must be untouched.
        assert!(client.challenge_take(others_id).await.is_some());
    }

    #[tokio::test]
    async fn friends_presence_first_and_last_tab_transitions() {
        let client = make_redis_client().await;
        let user = Uuid::new_v4();
        let session_a = format!("test:{}", Uuid::new_v4());
        let session_b = format!("test:{}", Uuid::new_v4());

        assert!(client.friends_mark_online(user, &session_a).await, "first tab");
        assert!(client.friends_is_online(user).await, "sanity: user is online");
        assert!(!client.friends_mark_online(user, &session_b).await, "second tab is not first");

        assert!(!client.friends_mark_offline(user, &session_a).await, "closing one of two tabs is not last");
        assert!(client.friends_is_online(user).await, "still online with one tab left");

        assert!(client.friends_mark_offline(user, &session_b).await, "closing the last tab");
        assert!(!client.friends_is_online(user).await, "offline once all tabs close");
    }

    #[tokio::test]
    async fn friends_presence_unknown_session_is_noop() {
        let client = make_redis_client().await;
        let user = Uuid::new_v4();
        let real_session = format!("test:{}", Uuid::new_v4());
        let stale_session = format!("test:{}", Uuid::new_v4());

        client.friends_mark_online(user, &real_session).await;
        let was_last = client.friends_mark_offline(user, &stale_session).await;
        assert!(!was_last, "a session that was never added must not report itself as last");
        assert!(client.friends_is_online(user).await, "real tab is still open");

        // cleanup
        client.friends_mark_offline(user, &real_session).await;
    }

    #[tokio::test]
    async fn mm_pending_match_round_trips_and_is_removed_on_take() {
        let client = make_redis_client().await;
        let user = Uuid::new_v4();
        let pending = PendingMatch { game_id: Uuid::new_v4(), side: Side::White };
        client.mm_set_pending_match(user, pending.clone()).await;

        let taken = client.mm_take_pending_match(user).await;
        assert_eq!(taken.map(|p| p.game_id), Some(pending.game_id));

        let taken_again = client.mm_take_pending_match(user).await;
        assert!(taken_again.is_none(), "double-take must not yield it twice");
    }

    #[tokio::test]
    async fn active_game_upsert_and_remove_round_trip() {
        let client = make_redis_client().await;
        let game_id = Uuid::new_v4();
        let white = Uuid::new_v4();
        let black = Uuid::new_v4();
        client.active_game_upsert(game_id, white, black, "blitz", true, "startpos", "test-instance").await;

        let found = client.active_games_excluding(&HashSet::new()).await;
        let entry = found.iter().find(|e| e.game_id == game_id).expect("game should be indexed");
        assert_eq!(entry.white_id, white);
        assert_eq!(entry.black_id, black);
        assert_eq!(entry.fen, "startpos");
        assert_eq!(client.active_game_owner(game_id).await.as_deref(), Some("test-instance"));

        client.active_game_update_fen(game_id, "moved").await;
        let found = client.active_games_excluding(&HashSet::new()).await;
        let entry = found.iter().find(|e| e.game_id == game_id).unwrap();
        assert_eq!(entry.fen, "moved");

        client.active_game_remove(game_id).await;
        let found = client.active_games_excluding(&HashSet::new()).await;
        assert!(found.iter().all(|e| e.game_id != game_id), "removed game must not be indexed");
        assert!(client.active_game_owner(game_id).await.is_none(), "owner lookup must miss once removed");
    }

    #[tokio::test]
    async fn active_game_upsert_sets_a_ttl_and_heartbeat_refreshes_it() {
        let client = make_redis_client().await;
        let game_id = Uuid::new_v4();
        client
            .active_game_upsert(game_id, Uuid::new_v4(), Uuid::new_v4(), "blitz", true, "startpos", "inst-a")
            .await;

        let key = format!("active_games:{game_id}");
        let ttl: i64 = client.pool.ttl(&key).await.unwrap();
        assert!(ttl > 0, "upsert must set a TTL, got {ttl}");

        // Manually shrink the TTL, then confirm the heartbeat call restores it —
        // this is the exact mechanism a healthy owning instance relies on to
        // keep a slow-clock game (long gaps between moves) from looking stale.
        let _: () = client.pool.expire(&key, 2, None).await.unwrap();
        client.refresh_active_game_heartbeat(game_id).await;
        let ttl_after: i64 = client.pool.ttl(&key).await.unwrap();
        assert!(ttl_after > 2, "heartbeat must refresh the TTL, got {ttl_after}");

        client.active_game_remove(game_id).await;
    }

    async fn insert_user(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, email) VALUES ($1, $2)",
            id,
            format!("{id}@test.invalid")
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    async fn insert_active_game(
        pool: &PgPool,
        game_id: Uuid,
        white_id: Uuid,
        black_id: Uuid,
        moves: &str,
        clocks: &str,
    ) {
        sqlx::query!(
            r#"INSERT INTO games (
                id, status, white_user_id, black_user_id, mode,
                time_initial_seconds, time_increment_seconds, rated, moves, clocks
            ) VALUES ($1, 'active', $2, $3, 'blitz', 180, 0, true, $4, $5)"#,
            game_id,
            white_id,
            black_id,
            moves,
            clocks,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn make_app_state(pool: PgPool) -> AppState {
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let config = Config::from_url(&redis_url).expect("invalid REDIS_URL");
        let redis_pool = Pool::new(config.clone(), None, None, None, 2).expect("build pool");
        redis_pool.connect();
        redis_pool.wait_for_connect().await.expect("Redis connect");
        let subscriber = fred::clients::SubscriberClient::new(config, None, None, None);
        let leptos_options = LeptosOptions::builder().output_name("test").build();
        AppState::new(leptos_options, pool, redis_pool, subscriber).await
    }

    /// The heartbeat-aware reaper, now adopt-before-abort: an ownerless game
    /// with real move history gets rebuilt and stays `active` (now owned by
    /// this instance); an ownerless game whose history doesn't replay is
    /// unadoptable and still gets aborted, exactly as before this feature.
    #[sqlx::test(migrations = "../../migrations")]
    async fn reconcile_adopts_adoptable_and_aborts_unadoptable(pool: PgPool) {
        let app_state = make_app_state(pool.clone()).await;

        let white_a = insert_user(&pool).await;
        let black_a = insert_user(&pool).await;
        let adoptable_id = Uuid::new_v4();
        insert_active_game(
            &pool,
            adoptable_id,
            white_a,
            black_a,
            "e2e4 e7e5",
            "178000,180000 178000,177500",
        )
        .await;

        let white_b = insert_user(&pool).await;
        let black_b = insert_user(&pool).await;
        let unadoptable_id = Uuid::new_v4();
        // e7e5 is illegal as White's first move → replay fails → unadoptable.
        insert_active_game(&pool, unadoptable_id, white_b, black_b, "e7e5", "178000,180000").await;

        // Neither game has a Redis ownership record — both look ownerless.
        let aborted_count = app_state.reconcile_stale_active_games().await.unwrap();
        assert_eq!(aborted_count, 1, "exactly the unadoptable game should be aborted");

        let adopted_status: String =
            sqlx::query_scalar!("SELECT status FROM games WHERE id = $1", adoptable_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(adopted_status, "active", "adopted game stays active, now owned by this instance");
        assert!(
            app_state.get_game_room(&adoptable_id).await.is_some(),
            "adopted room must be registered locally"
        );
        assert_eq!(
            app_state.redis_client.active_game_owner(adoptable_id).await.as_deref(),
            Some(instance_id()).as_deref(),
            "this instance must have claimed ownership"
        );

        let unadoptable_status: String =
            sqlx::query_scalar!("SELECT status FROM games WHERE id = $1", unadoptable_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(unadoptable_status, "aborted");

        app_state.redis_client.active_game_remove(adoptable_id).await;
    }

    /// The adoption compare-and-set: two instances racing to claim the same
    /// ownerless game must not both "win," or the two players end up on
    /// split-brain rooms on different instances — see `claim_game.lua`.
    #[tokio::test]
    async fn claim_active_game_is_exclusive() {
        let client = make_redis_client().await;
        let game_id = Uuid::new_v4();
        let white = Uuid::new_v4();
        let black = Uuid::new_v4();

        let first = client
            .claim_active_game(game_id, white, black, "blitz", true, "fen-a", "inst-a")
            .await;
        let second = client
            .claim_active_game(game_id, white, black, "blitz", true, "fen-b", "inst-b")
            .await;
        assert!(first, "first claim on an ownerless key must win");
        assert!(!second, "second claim must lose once the first has claimed it");
        assert_eq!(client.active_game_owner(game_id).await.as_deref(), Some("inst-a"));

        client.active_game_remove(game_id).await;
    }

    #[tokio::test]
    async fn claim_active_game_fails_once_a_real_owner_exists() {
        let client = make_redis_client().await;
        let game_id = Uuid::new_v4();
        client
            .active_game_upsert(game_id, Uuid::new_v4(), Uuid::new_v4(), "blitz", true, "startpos", "inst-a")
            .await;

        let claimed = client
            .claim_active_game(game_id, Uuid::new_v4(), Uuid::new_v4(), "blitz", true, "fen", "inst-b")
            .await;
        assert!(!claimed, "must not claim a game with a live owner");
        assert_eq!(client.active_game_owner(game_id).await.as_deref(), Some("inst-a"));

        client.active_game_remove(game_id).await;
    }

    /// End-to-end: publishing through `RedisClient` actually reaches a
    /// locally-registered inbox via the pattern-subscription dispatcher —
    /// not just that the Redis calls succeed in isolation. This is the
    /// highest-risk new code path in this phase (real pub/sub, not just
    /// key/value/set operations), so it gets its own direct test rather
    /// than only being exercised indirectly through higher-level call sites.
    #[tokio::test]
    async fn friends_publish_reaches_local_inbox_via_dispatcher() {
        let (client, friends_inboxes, _match_inboxes) = make_redis_client_with_inboxes().await;
        let user = Uuid::new_v4();
        let (tx, mut rx) = dummy_sender();
        friends_inbox_add(&friends_inboxes, user, Uuid::new_v4(), tx).await;

        client.friends_publish(user, &FriendsServerMessage::FriendListChanged).await;

        use futures::StreamExt;
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx.next())
            .await
            .expect("dispatcher did not deliver the published message in time");
        assert!(matches!(received, Some(Ok(FriendsServerMessage::FriendListChanged))));
    }

    #[tokio::test]
    async fn mm_publish_reaches_local_inbox_via_dispatcher() {
        let (client, _friends_inboxes, match_inboxes) = make_redis_client_with_inboxes().await;
        let user = Uuid::new_v4();
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Result<MatchmakingServerMessage, ServerFnError>>();
        match_inboxes.lock().await.insert(user, (Uuid::new_v4(), tx));

        let game_id = Uuid::new_v4();
        client
            .mm_publish(user, &MatchmakingServerMessage::Matched { game: game_id, side: Side::White })
            .await;

        use futures::StreamExt;
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx.next())
            .await
            .expect("dispatcher did not deliver the published message in time");
        assert!(matches!(received, Some(Ok(MatchmakingServerMessage::Matched { game, side: Side::White })) if game == game_id));
    }
}
