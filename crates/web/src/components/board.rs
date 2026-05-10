#[cfg(feature = "ssr")]
use crate::state::GameId;
#[cfg(feature = "ssr")]
use crate::state::GameRooms;

use leptos::{prelude::*, server};
use server_fn::{codec::JsonEncoding, BoxedStream, ServerFnError, Websocket};
use shakmaty::Board;
use shakmaty::Color;
use shakmaty::Piece;
use shakmaty::Square;
use shared::{ClientMessage, ServerMessage};
use uuid::Uuid;

#[server]
pub async fn get_board_state(game_id: Uuid) -> Result<String, ServerFnError> {
    use shakmaty::fen::Fen;

    let games = expect_context::<GameRooms>();
    let games = games.lock().await;
    if let Some(game_room) = games.get(&game_id) {
        let game_room = game_room.lock().await;
        let fen = Fen::from_position(&game_room.get_position(), shakmaty::EnPassantMode::Legal);
        Ok(fen.to_string())
    } else {
        Err(ServerFnError::new(format!("game {} not found", game_id)))
    }
}

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
async fn game_websocket(
    input: BoxedStream<ClientMessage, ServerFnError>,
) -> Result<BoxedStream<ServerMessage, ServerFnError>, ServerFnError> {
    use crate::game_room::GameRoom;
    use futures::StreamExt;
    use shakmaty::Position;
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
            let mut gr = game_room.lock().await;
            let role = gr.add_player(uuid);
            let fen = gr.get_position().board().board_fen().to_string();
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
                        let player_role = gr.add_player(uuid);
                        gr.broadcast(ServerMessage::UserJoined {
                            uuid,
                            position_fen: gr.get_position().board().board_fen().to_string(),
                            player_role,
                        });
                    }
                    ClientMessage::UserLeft { uuid } => todo!(),
                    ClientMessage::MoveMade { uci } => todo!(),
                    ClientMessage::Chat { user, text } => todo!(),
                }
            }
        }
    });

    Ok(rx.into())
}

#[component]
pub fn Square(rank: usize, file: usize, piece: Option<Piece>) -> impl IntoView {
    let image_path = piece.map(|piece| {
        let color = match piece.color {
            Color::White => "w",
            Color::Black => "b",
        };
        let role = match piece.role {
            shakmaty::Role::Pawn => "P",
            shakmaty::Role::Knight => "N",
            shakmaty::Role::Bishop => "B",
            shakmaty::Role::Rook => "R",
            shakmaty::Role::Queen => "Q",
            shakmaty::Role::King => "K",
        };
        format!("/piece/alpha/{}{}.svg", color, role)
    });

    view! {
        <div
            class="w-full h-full"
            class:bg-white=move || (rank + file) % 2 == 0
            class:bg-green-800=move || (rank + file) % 2 != 0
        >
            {image_path.map(|src| view! { <img src={src} class="w-full h-full"/> })}
        </div>
    }
}

#[component]
pub fn Board(game_id: Uuid) -> impl IntoView {
    use futures::channel::mpsc;
    use futures::StreamExt;
    use leptos::task::spawn_local;
    use shakmaty::fen::Fen;
    use shared::PlayerRole;

    let (mut tx, rx) = mpsc::channel(1);
    let (board, set_board) = signal(Board::default());
    let (selected_square, set_selected_square) = signal(None::<shakmaty::Square>);
    let (player_role, set_player_role) = signal(None::<PlayerRole>);

    leptos::logging::log!("Board mounted, hydrate={}", cfg!(feature = "hydrate"));
    if cfg!(feature = "hydrate") {
        spawn_local(async move {
            leptos::logging::log!("connecting websocket...");
            match game_websocket(rx.map(Ok).into()).await {
                Ok(mut messages) => {
                    leptos::logging::log!("websocket connected");
                    while let Some(msg) = messages.next().await {
                        if let Ok(msg) = msg {
                            match msg {
                                ServerMessage::UserJoined {
                                    uuid,
                                    position_fen,
                                    player_role,
                                } => {
                                    leptos::logging::log!(
                                        "User joined: uuid={}, fen={}, role={:?}",
                                        uuid,
                                        position_fen,
                                        player_role
                                    );
                                    let fen = position_fen.parse::<Fen>().unwrap();
                                    let setup = fen.as_setup();
                                    set_board.set(setup.board.clone());
                                }
                                ServerMessage::UserLeft { username } => todo!(),
                                ServerMessage::MoveMade { uci } => todo!(),
                                ServerMessage::Chat { user, text } => todo!(),
                            }
                        }
                    }
                    leptos::logging::log!("websocket stream ended");
                }
                Err(e) => leptos::logging::warn!("websocket error: {e}"),
            }
        });
        let send_result = tx.try_send(ClientMessage::UserJoined {
            uuid: Uuid::new_v4(),
            game_id,
        });
        leptos::logging::log!("try_send ok={}", send_result.is_ok());
    }

    view! {
        <div class="flex items-center justify-center w-full h-[calc(100vh-3.5rem)]">
            <div class="grid grid-cols-8 grid-rows-8 w-[min(100vw,calc(100vh-11.5rem))] aspect-square">
                <For
                    each={move || shakmaty::Square::ALL.into_iter().collect::<Vec<_>>()}
                    key=|sq: &shakmaty::Square| *sq as u8
                    let(sq)
                >
                    {
                        let rank = sq.rank().to_usize();
                        let file = sq.file().to_usize();

                        let piece = move || board.get().piece_at(sq);

                        view! {
                            <Square rank={rank} file={file} piece={piece()}/>
                        }
                    }
                </For>
            </div>
        </div>
    }
}
