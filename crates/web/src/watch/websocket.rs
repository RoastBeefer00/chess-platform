use leptos::prelude::*;
use server_fn::{codec::JsonEncoding, BoxedStream, Websocket};
use shared::{WatchClientMessage, WatchServerMessage};

/// How many games appear in the grid at once. A single source of truth for
/// both the cap itself and the "showing N of M active games" copy.
pub const WATCH_GRID_LIMIT: usize = 24;

/// How often the roster (which games are in the grid) is re-picked. Mirrors
/// lichess's TV-channel reselection cadence — fast enough to feel live,
/// slow enough that it's not a per-move cost.
#[cfg(feature = "ssr")]
const WATCH_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

/// Parses the lowercase category string stored in the cross-instance
/// `active_games:{id}` Redis hash (see `RedisClient::active_game_upsert`)
/// back into `Category`. `Category` has a `Display` impl for the other
/// direction but no `FromStr` — this is the one place that needs the
/// reverse, so it stays local rather than growing the shared crate's public
/// surface for a single call site.
#[cfg(feature = "ssr")]
fn parse_category(s: &str) -> Option<shared::Category> {
    match s {
        "bullet" => Some(shared::Category::Bullet),
        "blitz" => Some(shared::Category::Blitz),
        "rapid" => Some(shared::Category::Rapid),
        "classical" => Some(shared::Category::Classical),
        _ => None,
    }
}

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn watch_websocket(
    input: BoxedStream<WatchClientMessage, ServerFnError>,
) -> Result<BoxedStream<WatchServerMessage, ServerFnError>, ServerFnError> {
    use futures::StreamExt as _;
    use shakmaty::{fen::Fen, Color, EnPassantMode, Position as _};
    use shared::{Category, GameStatus, PlayerInfo, WatchGameSummary};
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    use tokio::sync::Mutex;
    use tokio_stream::{wrappers::BroadcastStream, StreamMap};
    use uuid::Uuid;

    use crate::auth::AuthBackend;
    use crate::game_room::GameRoom;
    use crate::state::AppState;
    use axum_login::AuthSession;

    fn now_ms() -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
    }

    /// Live remaining time for both sides, extrapolated the same way the
    /// game websocket's own join handler does: the stored `*_ms_left` is
    /// only true as of `last_move_at`, so the side to move needs whatever's
    /// elapsed since then subtracted out.
    fn live_clock_ms(gr: &GameRoom) -> (i64, i64) {
        let mut white_ms = gr.game.white_ms_left;
        let mut black_ms = gr.game.black_ms_left;
        if let Some(last) = gr.last_move_at {
            let elapsed = Instant::now().duration_since(last).as_millis() as i64;
            match gr.game.position.turn() {
                Color::White => white_ms = (white_ms - elapsed).max(0),
                Color::Black => black_ms = (black_ms - elapsed).max(0),
            }
        }
        (white_ms, black_ms)
    }

    /// `room: None` marks a remote candidate (owned by another instance) —
    /// `remote_fen` is only ever `Some` in that case, sourced from Redis
    /// since there's no local room to lock for a fresh one; the clock
    /// fields stay `None` for a remote candidate, since clocks aren't (yet)
    /// mirrored cross-instance.
    struct WatchCandidate {
        game_id: Uuid,
        room: Option<Arc<Mutex<GameRoom>>>,
        category: Category,
        rated: bool,
        white_id: Uuid,
        black_id: Uuid,
        remote_fen: Option<String>,
        white_ms_left: Option<i64>,
        black_ms_left: Option<i64>,
        /// When the clocks above were true. For a local room that's "now"
        /// (they're extrapolated at read time); for a remote one it's the
        /// timestamp the owning instance mirrored into Redis, which is what
        /// lets the client tick them down correctly rather than showing a
        /// figure that stopped being true when the roster tick ran.
        clock_sent_at_ms: Option<i64>,
    }

    struct ResolvedWatchCandidate {
        game_id: Uuid,
        room: Option<Arc<Mutex<GameRoom>>>,
        category: Category,
        rated: bool,
        white: PlayerInfo,
        black: PlayerInfo,
        remote_fen: Option<String>,
        white_ms_left: Option<i64>,
        black_ms_left: Option<i64>,
        clock_sent_at_ms: Option<i64>,
    }

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    // Real users and guests alike are ordinary `users` rows, so this gate
    // only turns away someone with no session at all — the one-click
    // "Continue as Guest" flow already covers everyone else.
    auth.user
        .ok_or_else(|| ServerFnError::new("unauthenticated"))?;
    let state = expect_context::<AppState>();

    let (tx, rx) =
        futures::channel::mpsc::unbounded::<Result<WatchServerMessage, ServerFnError>>();

    tokio::spawn(async move {
        let mut input = input;
        // game_id -> the room handle, kept alongside the subscription so a
        // move event can re-lock the same room to re-derive its FEN. Only
        // ever holds LOCALLY-owned rooms — a game owned by another instance
        // has no `GameRoom` here to lock, so it can't get the fast
        // subscription-driven update below; it still appears in the roster,
        // refreshed only on the periodic tick (its `fen` sourced from the
        // cross-instance Redis hash instead of a live lock).
        let mut rooms: std::collections::HashMap<Uuid, Arc<Mutex<GameRoom>>> =
            std::collections::HashMap::new();
        let mut moves: StreamMap<Uuid, BroadcastStream<shared::GameServerMessage>> =
            StreamMap::new();
        let mut refresh = tokio::time::interval(WATCH_REFRESH_INTERVAL);

        async fn game_fen(room: &Mutex<GameRoom>) -> String {
            let gr = room.lock().await;
            Fen::from_position(&gr.get_position(), EnPassantMode::Legal).to_string()
        }

        loop {
            tokio::select! {
                // Client disconnected.
                msg = input.next() => {
                    if msg.is_none() {
                        break;
                    }
                }

                _ = refresh.tick() => {
                    // Snapshot the map only long enough to clone the Arcs out —
                    // same shape as AppState::get_game_room.
                    let all_rooms: Vec<(Uuid, Arc<Mutex<GameRoom>>)> = {
                        let games = state.games.lock().await;
                        games.iter().map(|(id, r)| (*id, r.clone())).collect()
                    };

                    // `room: None` marks a remote candidate (owned by another
                    // instance) — its `fen` comes pre-populated from Redis
                    // since there's no local room to lock for a fresh one.
                    let mut local_ids: HashSet<Uuid> = HashSet::new();
                    let mut candidates: Vec<WatchCandidate> = Vec::new();
                    for (id, room) in &all_rooms {
                        let gr = room.lock().await;
                        if !matches!(gr.status, GameStatus::Ongoing) {
                            continue;
                        }
                        local_ids.insert(*id);
                        let (white_ms_left, black_ms_left) = live_clock_ms(&gr);
                        candidates.push(WatchCandidate {
                            game_id: *id,
                            room: Some(room.clone()),
                            category: gr.game.config.time_control.category(),
                            rated: gr.game.config.rated.is_rated(),
                            white_id: gr.game.white_player,
                            black_id: gr.game.black_player,
                            remote_fen: None,
                            white_ms_left: Some(white_ms_left),
                            black_ms_left: Some(black_ms_left),
                            clock_sent_at_ms: None,
                        });
                    }

                    for entry in state.redis_client.active_games_excluding(&local_ids).await {
                        let Some(category) = parse_category(&entry.category) else { continue };
                        candidates.push(WatchCandidate {
                            game_id: entry.game_id,
                            room: None,
                            category,
                            rated: entry.rated,
                            white_id: entry.white_id,
                            black_id: entry.black_id,
                            remote_fen: Some(entry.fen),
                            white_ms_left: entry.clocks.map(|c| c.white_ms_left),
                            black_ms_left: entry.clocks.map(|c| c.black_ms_left),
                            clock_sent_at_ms: entry.clocks.map(|c| c.sent_at_ms),
                        });
                    }
                    let total_active = candidates.len();

                    let resolved: Vec<ResolvedWatchCandidate> =
                        futures::future::join_all(candidates.into_iter().map(|c| {
                            let state = state.clone();
                            async move {
                                let (white, black) = tokio::try_join!(
                                    state.user_store.get_player_info(&c.white_id, c.category),
                                    state.user_store.get_player_info(&c.black_id, c.category),
                                )?;
                                Ok::<_, crate::auth::AuthError>(ResolvedWatchCandidate {
                                    game_id: c.game_id,
                                    room: c.room,
                                    category: c.category,
                                    rated: c.rated,
                                    white,
                                    black,
                                    remote_fen: c.remote_fen,
                                    white_ms_left: c.white_ms_left,
                                    black_ms_left: c.black_ms_left,
                                    clock_sent_at_ms: c.clock_sent_at_ms,
                                })
                            }
                        }))
                        .await
                        .into_iter()
                        .filter_map(|r| r.ok())
                        .collect();

                    let mut ranked = resolved;
                    ranked.sort_by_key(|c| std::cmp::Reverse(c.white.rating + c.black.rating));
                    ranked.truncate(WATCH_GRID_LIMIT);

                    // Diff subscriptions: drop games no longer in the roster,
                    // add newly-visible ones. Only ever touches LOCAL rooms —
                    // `moves`/`rooms` never gain an entry for a remote game.
                    let new_ids: HashSet<Uuid> = ranked.iter().map(|c| c.game_id).collect();
                    rooms.retain(|id, _| new_ids.contains(id));
                    let stale: Vec<Uuid> = moves
                        .keys()
                        .filter(|id| !new_ids.contains(id))
                        .copied()
                        .collect();
                    for id in stale {
                        moves.remove(&id);
                    }

                    let sent_at_ms = now_ms();
                    let mut summaries = Vec::with_capacity(ranked.len());
                    for c in ranked {
                        let fen = match &c.room {
                            Some(room) => {
                                if let std::collections::hash_map::Entry::Vacant(e) = rooms.entry(c.game_id) {
                                    e.insert(room.clone());
                                    moves.insert(c.game_id, BroadcastStream::new(room.lock().await.subscribe()));
                                }
                                game_fen(room).await
                            }
                            None => c.remote_fen.unwrap_or_default(),
                        };
                        summaries.push(WatchGameSummary {
                            game_id: c.game_id,
                            white: c.white,
                            black: c.black,
                            category: c.category,
                            rated: c.rated,
                            fen,
                            white_ms_left: c.white_ms_left,
                            black_ms_left: c.black_ms_left,
                            sent_at_ms: c.clock_sent_at_ms.unwrap_or(sent_at_ms),
                        });
                    }

                    if tx
                        .unbounded_send(Ok(WatchServerMessage::Roster {
                            games: summaries,
                            total_active,
                        }))
                        .is_err()
                    {
                        break;
                    }
                }

                Some((game_id, event)) = moves.next() => {
                    let Ok(event) = event else { continue };
                    let interesting = matches!(
                        event,
                        shared::GameServerMessage::MoveMade { .. }
                            | shared::GameServerMessage::Resync { .. }
                    );
                    if !interesting {
                        continue;
                    }
                    let Some(room) = rooms.get(&game_id) else { continue };
                    // A move just landed — clocks are fresh as of right now
                    // (no extrapolation needed, unlike the periodic tick).
                    let (fen, white_ms_left, black_ms_left) = {
                        let gr = room.lock().await;
                        (
                            Fen::from_position(&gr.get_position(), EnPassantMode::Legal).to_string(),
                            gr.game.white_ms_left,
                            gr.game.black_ms_left,
                        )
                    };
                    let sent_at_ms = now_ms();
                    if tx
                        .unbounded_send(Ok(WatchServerMessage::Position {
                            game_id,
                            fen,
                            white_ms_left,
                            black_ms_left,
                            sent_at_ms,
                        }))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });

    Ok(rx.into())
}
