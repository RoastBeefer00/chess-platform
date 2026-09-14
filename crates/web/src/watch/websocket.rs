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

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn watch_websocket(
    input: BoxedStream<WatchClientMessage, ServerFnError>,
) -> Result<BoxedStream<WatchServerMessage, ServerFnError>, ServerFnError> {
    use futures::StreamExt as _;
    use shakmaty::{fen::Fen, EnPassantMode};
    use shared::{Category, GameStatus, PlayerInfo, WatchGameSummary};
    use tokio::sync::Mutex;
    use tokio_stream::{wrappers::BroadcastStream, StreamMap};
    use uuid::Uuid;

    use crate::auth::AuthBackend;
    use crate::game_room::GameRoom;
    use crate::state::AppState;
    use axum_login::AuthSession;

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
        // move event can re-lock the same room to re-derive its FEN.
        let mut rooms: std::collections::HashMap<Uuid, std::sync::Arc<Mutex<GameRoom>>> =
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
                    let all_rooms: Vec<(Uuid, std::sync::Arc<Mutex<GameRoom>>)> = {
                        let games = state.games.lock().await;
                        games.iter().map(|(id, r)| (*id, r.clone())).collect()
                    };

                    let mut candidates = Vec::new();
                    for (id, room) in &all_rooms {
                        let gr = room.lock().await;
                        if !matches!(gr.status, GameStatus::Ongoing) {
                            continue;
                        }
                        candidates.push((
                            *id,
                            room.clone(),
                            gr.game.white_player,
                            gr.game.black_player,
                            gr.game.config.time_control.category(),
                            gr.game.config.rated.is_rated(),
                        ));
                    }
                    let total_active = candidates.len();

                    let resolved: Vec<(Uuid, std::sync::Arc<Mutex<GameRoom>>, Category, bool, PlayerInfo, PlayerInfo)> =
                        futures::future::join_all(candidates.into_iter().map(
                            |(id, room, white_id, black_id, category, rated)| {
                                let state = state.clone();
                                async move {
                                    let (white, black) = tokio::try_join!(
                                        state.user_store.get_player_info(&white_id, category.clone()),
                                        state.user_store.get_player_info(&black_id, category.clone()),
                                    )?;
                                    Ok::<_, crate::auth::AuthError>((id, room, category, rated, white, black))
                                }
                            },
                        ))
                        .await
                        .into_iter()
                        .filter_map(|r| r.ok())
                        .collect();

                    let mut ranked = resolved;
                    ranked.sort_by_key(|(_, _, _, _, white, black)| {
                        std::cmp::Reverse(white.rating + black.rating)
                    });
                    ranked.truncate(WATCH_GRID_LIMIT);

                    // Diff subscriptions: drop games no longer in the roster,
                    // add newly-visible ones.
                    let new_ids: std::collections::HashSet<Uuid> =
                        ranked.iter().map(|(id, ..)| *id).collect();
                    rooms.retain(|id, _| new_ids.contains(id));
                    let stale: Vec<Uuid> = moves
                        .keys()
                        .filter(|id| !new_ids.contains(id))
                        .copied()
                        .collect();
                    for id in stale {
                        moves.remove(&id);
                    }

                    let mut summaries = Vec::with_capacity(ranked.len());
                    for (id, room, category, rated, white, black) in ranked {
                        if !rooms.contains_key(&id) {
                            rooms.insert(id, room.clone());
                            moves.insert(id, BroadcastStream::new(room.lock().await.subscribe()));
                        }
                        let fen = game_fen(&room).await;
                        summaries.push(WatchGameSummary {
                            game_id: id,
                            white,
                            black,
                            category,
                            rated,
                            fen,
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
                    let fen = game_fen(room).await;
                    if tx
                        .unbounded_send(Ok(WatchServerMessage::Position { game_id, fen }))
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
