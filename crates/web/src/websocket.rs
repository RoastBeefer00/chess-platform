use leptos::prelude::*;
use server_fn::{codec::JsonEncoding, BoxedStream, Websocket};
use shared::{GameClientMessage, GameServerMessage, PlayerRole};

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn game_websocket(
    input: BoxedStream<GameClientMessage, ServerFnError>,
) -> Result<BoxedStream<GameServerMessage, ServerFnError>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::game_room::{handle_timeout, MoveError};
    use crate::state::AppState;
    use axum_login::AuthSession;
    use futures::StreamExt;
    use shakmaty::Position as _;
    use shared::messages::GameOverReason;
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
            receiver,
        ) = {
            use shakmaty::fen::Fen;
            let mut gr = game_room.lock().await;
            let role = gr.add_player(user.id);
            let fen =
                Fen::from_position(&gr.get_position(), shakmaty::EnPassantMode::Legal).to_string();

            // Compute live remaining time for the side-to-move (their clock has
            // been ticking since last_move_at on the server).
            let mut white_ms = gr.game.white_ms_left;
            let mut black_ms = gr.game.black_ms_left;
            if let Some(last) = gr.last_move_at {
                let elapsed = Instant::now().duration_since(last).as_millis() as i64;
                match gr.game.position.turn() {
                    shakmaty::Color::White => white_ms = (white_ms - elapsed).max(0),
                    shakmaty::Color::Black => black_ms = (black_ms - elapsed).max(0),
                }
            }

            (
                role,
                fen,
                white_ms,
                black_ms,
                gr.game.position.turn().into(),
                gr.last_move_at.is_some(),
                gr.subscribe(),
            )
        };

        let sent_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let _ = tx.unbounded_send(Ok(GameServerMessage::UserJoined {
            uuid: user.id,
            position_fen,
            player_role: player_role.clone(),
        }));

        let _ = tx.unbounded_send(Ok(GameServerMessage::ClockSync {
            white_ms_left,
            black_ms_left,
            turn,
            sent_at_ms,
            clock_running,
        }));

        let mut broadcast = BroadcastStream::new(receiver);
        let tx2 = tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = broadcast.next().await {
                let result = msg.map_err(|e| ServerFnError::new(e.to_string()));
                if tx2.unbounded_send(result).is_err() {
                    break;
                }
            }
        });

        while let Some(msg) = input.next().await {
            if let Ok(msg) = msg {
                tracing::debug!(?msg, "received message from client");
                let mut gr = game_room.lock().await;
                match msg {
                    GameClientMessage::UserJoined { game_id: _ } => {
                        tracing::warn!(%user.id, "ignoring duplicate UserJoined");
                    }
                    GameClientMessage::MoveMade { uci } => {
                        use crate::game_room::MoveOutcome;
                        match gr.handle_move_made(uci, user.id) {
                            Ok(MoveOutcome::Continuing(plan)) => {
                                if let Some(h) = gr.timeout_task.take() {
                                    h.abort();
                                }
                                let room = game_room.clone();
                                let handle = tokio::spawn(handle_timeout(
                                    room,
                                    plan.next_color,
                                    plan.ms_until_flag,
                                ));
                                gr.timeout_task = Some(handle);
                            }
                            Ok(MoveOutcome::Ended) => {
                                // end_game already broadcast + cancelled timer.
                            }
                            Err(MoveError::FlagFall) => {
                                // mover ran out applying their own move — they lose.
                                let winner_color = match gr.game.position.turn() {
                                    shakmaty::Color::White => shakmaty::Color::Black,
                                    shakmaty::Color::Black => shakmaty::Color::White,
                                };
                                gr.end_game(
                                    shakmaty::KnownOutcome::Decisive {
                                        winner: winner_color,
                                    },
                                    GameOverReason::Timeout,
                                );
                            }
                            Err(e) => tracing::warn!(?e, "move rejected"),
                        }
                    }
                    GameClientMessage::Chat { text: _ } => todo!(),
                    GameClientMessage::RematchOffer => {
                        if player_role == PlayerRole::Spectator {
                            return;
                        }

                        if let Some(id) = gr.rematch_offer {
                            if user.id == id {
                                return;
                            } else if gr.game.black_player == id {
                                let new_game_id = state
                                    .create_game(
                                        gr.game.config.clone(),
                                        gr.game.black_player,
                                        gr.game.white_player,
                                    )
                                    .await;
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

                        let new_game_id = state
                            .create_game(
                                gr.game.config.clone(),
                                gr.game.black_player,
                                gr.game.white_player,
                            )
                            .await;
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
                }
            }
        }
        let mut gr = game_room.lock().await;
        gr.remove_player(user.id);
        if gr.rematch_offer.is_some() {
            gr.clear_rematch_offer();
            gr.broadcast(GameServerMessage::RematchCancel);
        }
        gr.broadcast(GameServerMessage::UserLeft {
            username: user.username.unwrap_or_default(),
        });
    });

    Ok(rx.into())
}
