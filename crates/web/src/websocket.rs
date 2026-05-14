use leptos::prelude::*;
use server_fn::{codec::JsonEncoding, BoxedStream, Websocket};
use shared::{ClientMessage, ServerMessage};

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn game_websocket(
    input: BoxedStream<ClientMessage, ServerFnError>,
) -> Result<BoxedStream<ServerMessage, ServerFnError>, ServerFnError> {
    use crate::game_room::GameRoom;
    use crate::state::GameRooms;
    use futures::StreamExt;
    use shared::Game;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio_stream::wrappers::BroadcastStream;

    let mut input = input;
    let games = expect_context::<GameRooms>();

    // Return output channel immediately — don't block on input before returning
    // the stream. Blocking here causes a deadlock: the client won't send input
    // until game_websocket returns, but the server won't return until it reads input.
    let (tx, rx) = futures::channel::mpsc::unbounded::<Result<ServerMessage, ServerFnError>>();

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

        let (uuid, game_id) = match first {
            ClientMessage::UserJoined { uuid, game_id } => (uuid, game_id),
            _ => {
                let _ = tx.unbounded_send(Err(ServerFnError::new(
                    "expected UserJoined as first message",
                )));
                return;
            }
        };

        let game_room = games
            .lock()
            .await
            .entry(game_id)
            .or_insert_with(|| Arc::new(Mutex::new(GameRoom::new(Game::new(uuid, uuid)))))
            .clone();

        let (player_role, position_fen) = {
            use shakmaty::fen::Fen;
            let mut gr = game_room.lock().await;
            let role = gr.add_player(uuid);
            let fen =
                Fen::from_position(&gr.get_position(), shakmaty::EnPassantMode::Legal).to_string();
            (role, fen)
        };

        let _ = tx.unbounded_send(Ok(ServerMessage::UserJoined {
            uuid,
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
                println!("Received message from client: {:?}", msg);
                let mut gr = game_room.lock().await;
                match msg {
                    ClientMessage::UserJoined { uuid, game_id: _ } => {
                        use shakmaty::fen::Fen;
                        let player_role = gr.add_player(uuid);
                        let fen =
                            Fen::from_position(&gr.get_position(), shakmaty::EnPassantMode::Legal)
                                .to_string();
                        gr.broadcast(ServerMessage::UserJoined {
                            uuid,
                            position_fen: fen,
                            player_role,
                        });
                    }
                    ClientMessage::UserLeft { uuid } => todo!(),
                    ClientMessage::MoveMade { uci } => {
                        if gr.current_player() != Some(uuid) {
                            leptos::logging::warn!("move from non-current player");
                            continue;
                        }
                        use shakmaty::uci::UciMove;
                        let uci_move = match uci.parse::<UciMove>() {
                            Ok(u) => u,
                            Err(e) => {
                                leptos::logging::warn!("invalid uci: {e}");
                                continue;
                            }
                        };
                        let move_made = match uci_move.to_move(&gr.get_position()) {
                            Ok(m) => m,
                            Err(e) => {
                                leptos::logging::warn!("illegal move: {e}");
                                continue;
                            }
                        };
                        match gr.make_move(move_made) {
                            Ok(()) => {
                                gr.broadcast(ServerMessage::MoveMade { uci });
                            }
                            Err(e) => {
                                leptos::logging::warn!("failed to make move: {e}");
                            }
                        }
                    }
                    ClientMessage::Chat { user, text } => todo!(),
                }
            }
        }
    });

    Ok(rx.into())
}
