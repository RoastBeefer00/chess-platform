use leptos::prelude::*;
use server_fn::{codec::JsonEncoding, BoxedStream, Websocket};
use shared::{GameClientMessage, GameServerMessage};

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn game_websocket(
    input: BoxedStream<GameClientMessage, ServerFnError>,
) -> Result<BoxedStream<GameServerMessage, ServerFnError>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;
    use futures::StreamExt;
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

        let (player_role, position_fen) = {
            use shakmaty::fen::Fen;
            let mut gr = game_room.lock().await;
            let role = gr.add_player(user.id);
            let fen =
                Fen::from_position(&gr.get_position(), shakmaty::EnPassantMode::Legal).to_string();
            (role, fen)
        };

        let _ = tx.unbounded_send(Ok(GameServerMessage::UserJoined {
            uuid: user.id,
            position_fen,
            player_role,
        }));

        let mut broadcast = BroadcastStream::new(game_room.lock().await.subscribe());
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
                        if gr.current_player() != Some(user.id) {
                            tracing::warn!(%user.id, "move from non-current player");
                            continue;
                        }
                        use shakmaty::uci::UciMove;
                        let uci_move = match uci.parse::<UciMove>() {
                            Ok(u) => u,
                            Err(e) => {
                                tracing::warn!(%uci, %e, "invalid uci");
                                continue;
                            }
                        };
                        let move_made = match uci_move.to_move(&gr.get_position()) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::warn!(%uci, %e, "illegal move");
                                continue;
                            }
                        };
                        match gr.make_move(move_made) {
                            Ok(()) => {
                                tracing::info!(%uci, "move accepted");
                                gr.broadcast(GameServerMessage::MoveMade { uci });
                            }
                            Err(e) => {
                                tracing::warn!(%uci, %e, "failed to make move");
                            }
                        }
                    }
                    GameClientMessage::Chat { text } => todo!(),
                }
            }
        }
        let mut gr = game_room.lock().await;
        gr.remove_player(user.id);
        gr.broadcast(GameServerMessage::UserLeft {
            username: user.username.unwrap_or_default(),
        });
    });

    Ok(rx.into())
}
