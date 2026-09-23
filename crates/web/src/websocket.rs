use leptos::prelude::*;
use server_fn::{codec::JsonEncoding, BoxedStream, Websocket};
use shared::{GameClientMessage, GameServerMessage};

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn game_websocket(
    input: BoxedStream<GameClientMessage, ServerFnError>,
) -> Result<BoxedStream<GameServerMessage, ServerFnError>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::db::{finalize_now, GameOutcomeLookup};
    use crate::game_room::{handle_abort_timeout, handle_timeout, GameRoom};
    use crate::state::{AdoptOutcome, AppState};
    use axum_login::AuthSession;
    use futures::StreamExt;
    use shakmaty::{fen::Fen, EnPassantMode, Position as _};
    use shared::messages::GameOverReason;
    use shared::PlayerRole;
    use tokio_stream::wrappers::BroadcastStream;

    let mut input = input;
    let state = expect_context::<AppState>();
    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;

    // Return output channel immediately — don't block on input before returning
    // the stream. Blocking here causes a deadlock: the client won't send input
    // until game_websocket returns, but the server won't return until it reads input.
    let (tx, rx) = futures::channel::mpsc::unbounded::<Result<GameServerMessage, ServerFnError>>();

    let user = auth
        .user
        .ok_or_else(|| ServerFnError::new("unauthenticated"))?;

    /// Creates the rematch game and tells the room about it.
    ///
    /// Colors swap for the rematch, so the running series score has to swap
    /// with them: it's stored and rendered as `(white, black)` by board side,
    /// so carrying it across unchanged credits each player's wins to their
    /// opponent from the second game on.
    ///
    /// Returns `false` if game creation failed, leaving the offer standing so
    /// the players can simply try again.
    async fn accept_rematch(state: &AppState, gr: &mut GameRoom) -> bool {
        let (white_wins, black_wins) = gr.session_score;
        let new_game_id = match state
            .create_game(
                gr.game.config.clone(),
                gr.game.black_player,
                gr.game.white_player,
                (black_wins, white_wins),
            )
            .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(?e, "rematch: create_game failed");
                return false;
            }
        };
        gr.clear_rematch_offer();
        gr.broadcast(GameServerMessage::RematchAccept { new_game_id });
        true
    }

    tokio::spawn(async move {
        let first = match input.next().await {
            Some(Ok(msg)) => msg,
            Some(Err(e)) => {
                let _ = tx.unbounded_send(Err(ServerFnError::new(e.to_string())));
                return;
            }
            None => {
                let _ =
                    tx.unbounded_send(Err(ServerFnError::new("stream closed before UserJoined")));
                return;
            }
        };

        let game_id = match first {
            GameClientMessage::UserJoined { game_id } => game_id,
            _ => {
                let _ = tx.unbounded_send(Err(ServerFnError::new(
                    "expected UserJoined as first message",
                )));
                return;
            }
        };

        // Whether this join is riding in on a freshly-adopted room (see
        // `AppState::adopt_game`) — if so, the client may be holding moves
        // this rebuilt room doesn't have yet (progress persistence is
        // fire-and-forget), so a `Resync` gets pushed after the ordinary
        // join sequence below to make the server authoritative immediately.
        let mut adopted = false;
        let game_room = match state.get_game_room(&game_id).await {
            Some(room) => room,
            None => match state.adopt_game(game_id).await {
                AdoptOutcome::Adopted(room) => {
                    adopted = true;
                    room
                }
                AdoptOutcome::OwnedElsewhere => {
                    // A peer instance won the claim (race with this
                    // instance's own reaper sweep, or another reconnect).
                    // Fail this attempt — the client's own backoff/retry
                    // reconnects, and by then the fly-replay routing
                    // middleware sees the new owner and gets it right.
                    let _ = tx.unbounded_send(Err(ServerFnError::new(
                        "game claimed by another instance, reconnecting",
                    )));
                    return;
                }
                AdoptOutcome::Unadoptable => {
                    // No live GameRoom and nothing to adopt — either a bogus
                    // id, or a game that genuinely ended (or has
                    // unreplayable data) without this process ever having
                    // hosted it. Check the DB before giving up, so an ended
                    // game reconnects to a real GameOver replay instead of an
                    // undifferentiated error indistinguishable from a typo'd
                    // URL. This is also what makes evicting finished rooms
                    // safe (see `AppState::evict_finished_rooms`): once the
                    // room is gone, the DB row is the only thing left that
                    // knows how the game ended.
                    //
                    // The series score is reported as 0-0: it only ever lived
                    // in the room, so by here there's nothing left to report.
                    let outcome = state
                        .game_store
                        .get_finished_outcome(game_id)
                        .await
                        .unwrap_or(GameOutcomeLookup::Unknown);
                    let message = match outcome {
                        GameOutcomeLookup::Ended { winner, reason } => {
                            Ok(GameServerMessage::GameOver {
                                winner,
                                reason,
                                white_wins: 0.0,
                                black_wins: 0.0,
                            })
                        }
                        GameOutcomeLookup::Unknown | GameOutcomeLookup::StillRunning => {
                            Err(ServerFnError::new("game not found"))
                        }
                    };
                    let _ = tx.unbounded_send(message);
                    return;
                }
            },
        };

        use std::time::{Instant, SystemTime, UNIX_EPOCH};
        let (
            player_role,
            position_fen,
            white_ms_left,
            black_ms_left,
            turn,
            clock_running,
            move_history,
            finished_replay,
            session_score,
            abort_countdown_snapshot,
            receiver,
            player_side,
            opponent_presence_snapshot,
        ) = {
            use shakmaty::fen::Fen;
            let mut gr = game_room.lock().await;
            let (role, started) = gr.add_player(user.id);

            if started {
                gr.start_abort_window(shared::Side::White);
                let handle = tokio::spawn(handle_abort_timeout(
                    game_room.clone(),
                    state.game_store.clone(),
                    state.redis_client.clone(),
                    shakmaty::Color::White,
                ));
                gr.abort_task = Some(handle);
            }

            // Inform the room that this player has connected/reconnected.
            let joining_side: Option<shared::Side> = match &role {
                shared::PlayerRole::Player(side) => Some(*side),
                shared::PlayerRole::Spectator => None,
            };
            if let Some(side) = joining_side {
                gr.broadcast(GameServerMessage::PresenceUpdate {
                    side,
                    connected: true,
                    rtt_ms: None,
                });
            }
            // Snapshot opponent's current presence to send just to this client
            // (the broadcast above only goes to already-subscribed clients).
            let opponent_presence_snapshot: Option<GameServerMessage> = {
                let opp_id = if user.id == gr.game.white_player {
                    Some(gr.game.black_player)
                } else if user.id == gr.game.black_player {
                    Some(gr.game.white_player)
                } else {
                    None
                };
                opp_id.and_then(|id| {
                    if gr.connected.contains_key(&id) {
                        gr.user_side(id).map(|side| GameServerMessage::PresenceUpdate {
                            side,
                            connected: true,
                            rtt_ms: gr.rtt_of(id),
                        })
                    } else {
                        None
                    }
                })
            };
            let fen =
                Fen::from_position(&gr.get_position(), shakmaty::EnPassantMode::Legal).to_string();

            // If the game already finished, capture the outcome+reason so we can
            // replay GameOver to this reconnecting client (mobile WS suspension
            // commonly causes the original broadcast to be missed).
            let finished_replay: Option<(Option<shared::Side>, GameOverReason)> = match &gr.status {
                shared::GameStatus::Finished(shakmaty::Outcome::Known(known)) => {
                    let winner = match known {
                        shakmaty::KnownOutcome::Decisive { winner } => {
                            Some(shared::Side::from(*winner))
                        }
                        shakmaty::KnownOutcome::Draw => None,
                    };
                    // `end_game` is the only thing that sets `Finished`, and
                    // it always sets `end_reason` alongside — so `None` here
                    // means the two drifted apart. Skip the replay rather
                    // than invent a reason and show the player something
                    // confidently wrong.
                    match gr.end_reason.clone() {
                        Some(reason) => Some((winner, reason)),
                        None => {
                            tracing::warn!(%game_id, "finished room has no end_reason");
                            None
                        }
                    }
                }
                _ => None,
            };

            // Compute live remaining time for the side-to-move (their clock has
            // been ticking since last_move_at on the server). Skip when the game
            // is finished — the stored values are already the final snapshot.
            let mut white_ms = gr.game.white_ms_left;
            let mut black_ms = gr.game.black_ms_left;
            if finished_replay.is_none() {
                if let Some(last) = gr.last_move_at {
                    let elapsed = Instant::now().duration_since(last).as_millis() as i64;
                    match gr.game.position.turn() {
                        shakmaty::Color::White => white_ms = (white_ms - elapsed).max(0),
                        shakmaty::Color::Black => black_ms = (black_ms - elapsed).max(0),
                    }
                }
            }

            (
                role,
                fen,
                white_ms,
                black_ms,
                gr.game.position.turn().into(),
                finished_replay.is_none() && gr.last_move_at.is_some(),
                gr.move_history.clone(),
                finished_replay,
                gr.session_score,
                (gr.abort_side, gr.abort_deadline_ms),
                gr.subscribe(),
                joining_side,
                opponent_presence_snapshot,
            )
        };

        let sent_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let (white_wins, black_wins) = session_score;
        let _ = tx.unbounded_send(Ok(GameServerMessage::UserJoined {
            uuid: user.id,
            position_fen,
            player_role: player_role.clone(),
            moves: move_history,
            white_wins,
            black_wins,
        }));

        let _ = tx.unbounded_send(Ok(GameServerMessage::ClockSync {
            white_ms_left,
            black_ms_left,
            turn,
            sent_at_ms,
            clock_running,
        }));

        // Reconnect: if an abort window was active, restore the countdown for
        // this client (broadcast already fired when the window started, but
        // this client joined late or reconnected).
        if let (Some(side), Some(deadline_ms)) = abort_countdown_snapshot {
            let _ = tx.unbounded_send(Ok(GameServerMessage::AbortCountdown {
                side: Some(side),
                deadline_ms: Some(deadline_ms),
            }));
        }

        if let Some((winner, reason)) = finished_replay {
            let _ = tx.unbounded_send(Ok(GameServerMessage::GameOver {
                winner,
                reason,
                white_wins,
                black_wins,
            }));
        }

        // Push opponent's current presence directly to this client (they missed
        // the broadcast that fired when the opponent first connected).
        if let Some(presence) = opponent_presence_snapshot {
            let _ = tx.unbounded_send(Ok(presence));
        }

        // Adopted room: the client may hold moves this rebuilt room never
        // received (progress persistence is fire-and-forget), so make the
        // server's rebuilt state authoritative immediately rather than
        // waiting for the client's next move to be rejected.
        if adopted {
            let resync = game_room.lock().await.build_resync();
            let _ = tx.unbounded_send(Ok(resync));
        }

        let mut broadcast = BroadcastStream::new(receiver);
        let tx2 = tx.clone();
        let room_for_lag = game_room.clone();
        tokio::spawn(async move {
            while let Some(msg) = broadcast.next().await {
                match msg {
                    Ok(m) => {
                        if tx2.unbounded_send(Ok(m)).is_err() {
                            break;
                        }
                    }
                    Err(_lagged) => {
                        // Subscriber fell behind the broadcast buffer. Without
                        // this, the client silently misses moves and desyncs
                        // (highlight without piece movement). Push an
                        // authoritative snapshot so the client recovers.
                        let resync = room_for_lag.lock().await.build_resync();
                        if tx2.unbounded_send(Ok(resync)).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // Running minimum of (server_recv_ms - client_send_ms) over Pings.
        // min(recv - send) ≈ true clock offset (best-case one-way lag → 0).
        // Connection-local so the lock-free Ping fast-path stays lock-free.
        let mut offset_est: Option<i64> = None;
        // Last RTT bucket broadcast for this connection; used to suppress
        // redundant PresenceUpdate broadcasts in the steady state.
        let mut prev_bucket: Option<u8> = None;

        while let Some(msg) = input.next().await {
            if let Ok(msg) = msg {
                tracing::debug!(?msg, "received message from client");
                // Clock-offset probe: pure timestamp echo, no game state. Handle
                // before taking the game-room lock so a flood of pings can't
                // contend on the mutex with actual gameplay.
                if let GameClientMessage::Ping { client_time_ms } = &msg {
                    let client_time_ms = *client_time_ms;
                    let server_time_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(0);
                    let d = server_time_ms - client_time_ms;
                    let new_offset = offset_est.map_or(d, |e| e.min(d));
                    offset_est = Some(new_offset);
                    // RTT estimate: 2 * (server_recv - client_send - min_offset).
                    // Lock only on bucket transitions to preserve the fast path.
                    if let Some(side) = player_side {
                        use shared::rtt_bucket;
                        let rtt_est = ((server_time_ms - client_time_ms - new_offset) * 2)
                            .clamp(0, 60_000) as u32;
                        let new_bucket = rtt_bucket(rtt_est);
                        if prev_bucket != Some(new_bucket) {
                            prev_bucket = Some(new_bucket);
                            let mut gr = game_room.lock().await;
                            gr.update_rtt(user.id, rtt_est);
                            gr.broadcast(GameServerMessage::PresenceUpdate {
                                side,
                                connected: true,
                                rtt_ms: Some(rtt_est),
                            });
                        }
                    }
                    let _ = tx.unbounded_send(Ok(GameServerMessage::Pong {
                        client_time_ms,
                        server_time_ms,
                    }));
                    continue;
                }
                let mut gr = game_room.lock().await;
                match msg {
                    GameClientMessage::UserJoined { game_id: _ } => {
                        tracing::warn!(%user.id, "ignoring duplicate UserJoined");
                    }
                    GameClientMessage::MoveMade { uci, think_ms } => {
                        use crate::game_room::MoveOutcome;
                        match gr.handle_move_made(uci, user.id, think_ms) {
                            Ok(MoveOutcome::Continuing(plan)) => {
                                crate::db::spawn_progress_persist(
                                    state.game_store.clone(),
                                    game_id,
                                    gr.move_history.clone(),
                                    gr.clock_history.clone(),
                                );
                                // Fire-and-forget, same as the persist call above —
                                // never adds latency to the move reaching the
                                // opponent (that's the instant in-memory broadcast).
                                let fen = Fen::from_position(&gr.get_position(), EnPassantMode::Legal).to_string();
                                // The clocks ride along with the position so
                                // a watch grid on another instance can tick
                                // this game down — it has no `GameRoom` here
                                // to read them off. Accurate as of right now:
                                // the mover's clock was just charged.
                                let clocks = crate::state::ActiveGameClocks {
                                    white_ms_left: gr.game.white_ms_left,
                                    black_ms_left: gr.game.black_ms_left,
                                    sent_at_ms: crate::state::now_epoch_ms(),
                                };
                                let redis_client = state.redis_client.clone();
                                tokio::spawn(async move {
                                    redis_client
                                        .active_game_update_position(game_id, &fen, clocks)
                                        .await;
                                });
                                if let Some(h) = gr.timeout_task.take() {
                                    h.abort();
                                }
                                let room = game_room.clone();
                                let handle = tokio::spawn(handle_timeout(
                                    room,
                                    state.game_store.clone(),
                                    state.redis_client.clone(),
                                    plan.next_color,
                                    plan.ms_until_flag,
                                ));
                                gr.timeout_task = Some(handle);

                                // Abort window transitions based on move count.
                                match gr.move_history.len() {
                                    1 => {
                                        // White just moved — cancel white's abort task
                                        // and start black's window.
                                        if let Some(h) = gr.abort_task.take() {
                                            h.abort();
                                        }
                                        gr.start_abort_window(shared::Side::Black);
                                        let handle = tokio::spawn(handle_abort_timeout(
                                            game_room.clone(),
                                            state.game_store.clone(),
                                            state.redis_client.clone(),
                                            shakmaty::Color::Black,
                                        ));
                                        gr.abort_task = Some(handle);
                                    }
                                    2 => {
                                        // Black just moved — both have moved; no more abort.
                                        gr.clear_abort_window();
                                    }
                                    _ => {}
                                }
                            }
                            Ok(MoveOutcome::Ended(plan)) => {
                                // end_game already broadcast + cancelled timer.
                                // Awaited, not detached — see `finalize_now`'s
                                // doc comment for why that matters. Drop the
                                // room lock first; nothing below needs it and
                                // there's no reason to hold it for a DB round trip.
                                drop(gr);
                                finalize_now(&state.game_store, &state.redis_client, plan).await;
                            }
                            // A flag fall while applying the mover's own move
                            // now resolves inside `handle_move_made` and
                            // arrives as `Ended` above — it has the mover's
                            // color to hand, which this arm did not.
                            Err(e) => {
                                tracing::warn!(?e, "move rejected");
                                // Client optimistically applied this move
                                // locally; without telling them we rejected
                                // it they stay diverged until refresh. Push
                                // an authoritative snapshot just to this
                                // client.
                                let _ = tx.unbounded_send(Ok(gr.build_resync()));
                            }
                        }
                    }
                    // Not built yet. `todo!()` here used to panic the whole
                    // session task on any client that sent one — and this
                    // variant deserializes straight off the wire, so that
                    // was reachable by anyone.
                    GameClientMessage::Chat { text: _ } => {
                        tracing::debug!("ignoring Chat: not implemented");
                    }
                    GameClientMessage::Resign => {
                        let my_side = match &player_role {
                            shared::PlayerRole::Player(side) => *side,
                            shared::PlayerRole::Spectator => continue,
                        };
                        if !matches!(gr.status, shared::GameStatus::Ongoing) {
                            continue;
                        }
                        let winner_color: shakmaty::Color = my_side.opposite().into();
                        let plan = gr.end_game(
                            shakmaty::KnownOutcome::Decisive {
                                winner: winner_color,
                            },
                            GameOverReason::Resignation,
                        );
                        drop(gr);
                        finalize_now(&state.game_store, &state.redis_client, plan).await;
                    }
                    GameClientMessage::DrawOffer => {
                        if !matches!(player_role, shared::PlayerRole::Player(_)) {
                            continue;
                        }
                        if !matches!(gr.status, shared::GameStatus::Ongoing) {
                            continue;
                        }
                        if gr.draw_offer.is_some() {
                            continue;
                        }
                        gr.draw_offer = Some(user.id);
                        gr.broadcast(GameServerMessage::DrawOffer { from: user.id });
                    }
                    GameClientMessage::DrawAccept => {
                        if !matches!(player_role, shared::PlayerRole::Player(_)) {
                            continue;
                        }
                        if let Some(offerer) = gr.draw_offer {
                            if offerer != user.id {
                                gr.clear_draw_offer();
                                let plan =
                                    gr.end_game(shakmaty::KnownOutcome::Draw, GameOverReason::DrawAgreement);
                                drop(gr);
                                finalize_now(&state.game_store, &state.redis_client, plan).await;
                            }
                        }
                    }
                    GameClientMessage::DrawDecline => {
                        if !matches!(player_role, shared::PlayerRole::Player(_)) {
                            continue;
                        }
                        if let Some(offerer) = gr.draw_offer {
                            if offerer != user.id {
                                gr.clear_draw_offer();
                                gr.broadcast(GameServerMessage::DrawDecline);
                            }
                        }
                    }
                    // A rematch starts a whole new rated game, so every arm
                    // below gates on being a player in a game that's actually
                    // over. Without the `Finished` check a mid-game rematch
                    // would leave the user with two `status='active'` rows,
                    // which breaks the "at most one active game per user"
                    // assumption `find_active_game` is built on.
                    GameClientMessage::RematchOffer => {
                        if player_role == PlayerRole::Spectator
                            || !matches!(gr.status, shared::GameStatus::Finished(_))
                        {
                            continue;
                        }
                        match gr.rematch_offer {
                            // Re-offering our own outstanding offer: nothing
                            // to do. (This used to `return`, which killed the
                            // whole session and skipped the disconnect
                            // cleanup below, leaving the player's presence
                            // refcount stuck "connected" forever.)
                            Some(id) if id == user.id => {}
                            // The opponent already offered, so this is an
                            // implicit accept — regardless of which color
                            // they happen to be playing.
                            Some(_) => {
                                accept_rematch(&state, &mut gr).await;
                            }
                            None => {
                                gr.rematch_offer = Some(user.id);
                                gr.broadcast(GameServerMessage::RematchOffer { from: user.id });
                            }
                        }
                    }
                    GameClientMessage::RematchAccept => {
                        if player_role == PlayerRole::Spectator
                            || !matches!(gr.status, shared::GameStatus::Finished(_))
                        {
                            continue;
                        }
                        // There must be an outstanding offer, and it must be
                        // the opponent's. Without this an accept was
                        // unconditional, so a client could spam it to mint
                        // unlimited games off a single finished room.
                        match gr.rematch_offer {
                            Some(id) if id != user.id => {
                                accept_rematch(&state, &mut gr).await;
                            }
                            _ => {}
                        }
                    }
                    GameClientMessage::RematchDecline => {
                        if player_role == PlayerRole::Spectator {
                            continue;
                        }
                        if let Some(id) = gr.rematch_offer {
                            if user.id != id {
                                gr.clear_rematch_offer();
                                gr.broadcast(GameServerMessage::RematchDecline);
                            }
                        }
                    }
                    GameClientMessage::RematchCancel => {
                        if player_role == PlayerRole::Spectator {
                            continue;
                        }
                        if let Some(id) = gr.rematch_offer {
                            if user.id == id {
                                gr.clear_rematch_offer();
                                gr.broadcast(GameServerMessage::RematchCancel);
                            }
                        }
                    }
                    // Handled before the lock above; never reached here.
                    GameClientMessage::Ping { .. } => {}
                }
            }
        }
        let mut gr = game_room.lock().await;
        let disconnecting_side = gr.user_side(user.id);
        gr.remove_player(user.id);
        if !gr.connected.contains_key(&user.id) {
            if let Some(side) = disconnecting_side {
                gr.broadcast(GameServerMessage::PresenceUpdate {
                    side,
                    connected: false,
                    rtt_ms: None,
                });
            }
        }
        if gr.rematch_offer.is_some() {
            gr.clear_rematch_offer();
            gr.broadcast(GameServerMessage::RematchCancel);
        }
        if gr.draw_offer.is_some() {
            gr.clear_draw_offer();
            gr.broadcast(GameServerMessage::DrawDecline);
        }
        gr.broadcast(GameServerMessage::UserLeft {
            username: user.username.unwrap_or_default(),
        });
    });

    Ok(rx.into())
}
