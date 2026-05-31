use leptos::prelude::*;
use server_fn::{codec::JsonEncoding, BoxedStream, Websocket};
use shared::{GameClientMessage, GameServerMessage};

#[cfg(feature = "ssr")]
const LAG_CAP_MS: i64 = 100;

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn game_websocket(
    input: BoxedStream<GameClientMessage, ServerFnError>,
) -> Result<BoxedStream<GameServerMessage, ServerFnError>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::db::spawn_finalize;
    use crate::game_room::{handle_abort_timeout, handle_timeout, MoveError};
    use crate::state::AppState;
    use axum_login::AuthSession;
    use futures::StreamExt;
    use shakmaty::Position as _;
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

        let Some(game_room) = state.get_game_room(&game_id).await else {
            let _ = tx.unbounded_send(Err(ServerFnError::new("game not found")));
            return;
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
        ) = {
            use shakmaty::fen::Fen;
            let mut gr = game_room.lock().await;
            let (role, started) = gr.add_player(user.id);

            if started {
                gr.start_abort_window(shared::Side::White);
                let handle = tokio::spawn(handle_abort_timeout(
                    game_room.clone(),
                    state.game_store.clone(),
                    shakmaty::Color::White,
                ));
                gr.abort_task = Some(handle);
            }
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
                    let reason = gr.end_reason.clone().unwrap_or(GameOverReason::Draw);
                    Some((winner, reason))
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
                    offset_est = Some(offset_est.map_or(d, |e| e.min(d)));
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
                    GameClientMessage::MoveMade { uci, client_time_ms } => {
                        let lag = match offset_est {
                            Some(offset) => {
                                let recv = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .map(|d| d.as_millis() as i64)
                                    .unwrap_or(0);
                                (recv - (client_time_ms + offset)).clamp(0, LAG_CAP_MS)
                            }
                            None => 0,
                        };
                        use crate::game_room::MoveOutcome;
                        match gr.handle_move_made(uci, user.id, lag) {
                            Ok(MoveOutcome::Continuing(plan)) => {
                                if let Some(h) = gr.timeout_task.take() {
                                    h.abort();
                                }
                                let room = game_room.clone();
                                let handle = tokio::spawn(handle_timeout(
                                    room,
                                    state.game_store.clone(),
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
                                spawn_finalize(state.game_store.clone(), plan);
                            }
                            Err(MoveError::FlagFall) => {
                                // mover ran out applying their own move — they lose.
                                let winner_color = match gr.game.position.turn() {
                                    shakmaty::Color::White => shakmaty::Color::Black,
                                    shakmaty::Color::Black => shakmaty::Color::White,
                                };
                                let plan = gr.end_game(
                                    shakmaty::KnownOutcome::Decisive {
                                        winner: winner_color,
                                    },
                                    GameOverReason::Timeout,
                                );
                                spawn_finalize(state.game_store.clone(), plan);
                            }
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
                    GameClientMessage::Chat { text: _ } => todo!(),
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
                        spawn_finalize(state.game_store.clone(), plan);
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
                                    gr.end_game(shakmaty::KnownOutcome::Draw, GameOverReason::Draw);
                                spawn_finalize(state.game_store.clone(), plan);
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
                    GameClientMessage::RematchOffer => {
                        if player_role == PlayerRole::Spectator {
                            return;
                        }

                        if let Some(id) = gr.rematch_offer {
                            if user.id == id {
                                return;
                            } else if gr.game.black_player == id {
                                let score = gr.session_score;
                                let new_game_id = match state
                                    .create_game(
                                        gr.game.config.clone(),
                                        gr.game.black_player,
                                        gr.game.white_player,
                                        score,
                                    )
                                    .await
                                {
                                    Ok(id) => id,
                                    Err(e) => {
                                        tracing::warn!(?e, "rematch: create_game failed");
                                        continue;
                                    }
                                };
                                gr.broadcast(GameServerMessage::RematchAccept { new_game_id });
                                gr.clear_rematch_offer();
                            }
                        } else {
                            gr.rematch_offer = Some(user.id);
                            gr.broadcast(GameServerMessage::RematchOffer { from: user.id });
                        }
                    }
                    GameClientMessage::RematchAccept => {
                        if player_role == PlayerRole::Spectator {
                            return;
                        }

                        let score = gr.session_score;
                        let new_game_id = match state
                            .create_game(
                                gr.game.config.clone(),
                                gr.game.black_player,
                                gr.game.white_player,
                                score,
                            )
                            .await
                        {
                            Ok(id) => id,
                            Err(e) => {
                                tracing::warn!(?e, "rematch: create_game failed");
                                continue;
                            }
                        };
                        gr.broadcast(GameServerMessage::RematchAccept { new_game_id });
                        gr.clear_rematch_offer();
                    }
                    GameClientMessage::RematchDecline => {
                        if player_role == PlayerRole::Spectator {
                            return;
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
                            return;
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
        gr.remove_player(user.id);
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
