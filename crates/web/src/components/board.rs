#[cfg(feature = "ssr")]
use crate::state::GameId;
#[cfg(feature = "ssr")]
use crate::state::GameRooms;

use leptos::attr::Allowfullscreen;
use leptos::html::Div;
use leptos::html::Img;
use leptos::{prelude::*, server};
use leptos_use::use_draggable;
use leptos_use::UseDraggableReturn;
use serde::Deserialize;
use serde::Serialize;
use server_fn::{codec::JsonEncoding, BoxedStream, ServerFnError, Websocket};
use shakmaty::Board;
use shakmaty::Color;
use shakmaty::Move;
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
                    ClientMessage::MoveMade { uci } => todo!(),
                    ClientMessage::Chat { user, text } => todo!(),
                }
            }
        }
    });

    Ok(rx.into())
}

#[component]
pub fn Square(
    rank: usize,
    file: usize,
    piece: Option<Piece>,
    perspective: BoardPerspective,
) -> impl IntoView {
    let selected_square = expect_context::<RwSignal<Option<shakmaty::Square>>>();
    let valid_move_targets = expect_context::<Signal<Vec<shakmaty::Square>>>();

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

    let el = NodeRef::<Img>::new();

    #[cfg(feature = "hydrate")]
    let UseDraggableReturn {
        is_dragging, style, ..
    } = {
        use leptos_use::core::Position;
        use leptos_use::{
            use_draggable_with_options, UseDraggableCallbackArgs, UseDraggableOptions,
        };

        let pos = RwSignal::new(Position { x: 0.0, y: 0.0 });

        use_draggable_with_options(
            el,
            UseDraggableOptions::default()
                .initial_value(pos)
                .on_start(move |_: UseDraggableCallbackArgs| {
                    selected_square.set(Some(Square::new((rank * 8 + file) as u32)));
                    if let Some(element) = el.get_untracked() {
                        let rect = element.get_bounding_client_rect();
                        pos.set(Position {
                            x: rect.left(),
                            y: rect.top(),
                        });
                    }
                    true
                })
                .on_end(move |args: UseDraggableCallbackArgs| {
                    // TODO: Get target square from drop position
                    let (x, y) = (args.event.client_x() as f32, args.event.client_y() as f32);
                    let dom_element = web_sys::window()
                        .unwrap()
                        .document()
                        .unwrap()
                        .element_from_point(x, y)
                        .unwrap();

                    let data = dom_element.closest("[data-square]").unwrap();
                    if let Some(el) = data {
                        let attr = el.get_attribute("data-square").unwrap();
                        leptos::logging::log!("dropped on square {}", attr);
                    } else {
                        leptos::logging::log!("dropped outside of board");
                    }

                    // TODO: Check if move is legal
                    // TODO: Replace square with target square if legal

                    // Unset selected square
                    selected_square.set(None);
                    // TODO: Send move to server
                }),
        )
    };

    fn rank_to_char(rank: usize) -> char {
        std::char::from_digit((rank + 1) as u32, 10).unwrap()
    }

    fn file_to_char(file: usize) -> char {
        (b'a' + file as u8) as char
    }

    #[cfg(not(feature = "hydrate"))]
    let (is_dragging, style) = (Signal::derive(|| false), Signal::derive(|| String::new()));

    view! {
        <div
            class="relative w-full h-full"
            class:bg-white=move || (rank + file) % 2 == 0
            class:bg-green-800=move || (rank + file) % 2 != 0
            data-square=format!("{}{}", file_to_char(file), rank_to_char(rank))
        >
            <Show
                when=move || valid_move_targets.get().contains(&Square::new((rank * 8 + file) as u32))
            >
                {if piece.is_some() {
                    view! {
                        <div class="absolute inset-0 rounded-full ring-[6px] ring-inset ring-black opacity-20 pointer-events-none z-10"></div>
                    }.into_any()
                } else {
                    view! {
                        <div class="absolute inset-0 flex items-center justify-center pointer-events-none z-10">
                            <div class="w-1/3 h-1/3 rounded-full bg-black opacity-20"></div>
                        </div>
                    }.into_any()
                }}
            </Show>
            {image_path.map(|src| view! {
                <img
                    src={src}
                    node_ref=el
                    class="w-full h-full cursor-grab"
                    class:cursor-grabbing=move || is_dragging.get()
                    class:opacity-50=move || is_dragging.get()
                    style=move || if is_dragging.get() {
                        format!("position: fixed; {} pointer-events: none; width: 80px; height: 80px; z-index: 50;", style.get())
                    } else {
                        String::new()
                    }
                />
            })}
            <Show when=move || perspective == BoardPerspective::White && rank == 0 || perspective == BoardPerspective::Black && rank == 7>
                <span
                    class="absolute bottom-0 left-0.5 font-bold text-sm"
                    class:text-white=move || (rank + file) % 2 != 0
                    class:text-green-800=move || (rank + file) % 2 == 0
                >{file_to_char(file)}</span>
            </Show>
            <Show when=move || perspective == BoardPerspective::White && file == 7 || perspective == BoardPerspective::Black && file == 0>
                <span
                    class="absolute top-0 right-0.5 font-bold text-sm"
                    class:text-white=move || (rank + file) % 2 != 0
                    class:text-green-800=move || (rank + file) % 2 == 0
                >{rank_to_char(rank)}</span>
            </Show>
        </div>
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardPerspective {
    White,
    Black,
}

#[island]
pub fn Board(game_id: Uuid, perspective: BoardPerspective) -> impl IntoView {
    use futures::channel::mpsc;
    use futures::StreamExt;
    use leptos::task::spawn_local;
    use shakmaty::fen::Fen;
    use shared::PlayerRole;

    let (mut tx, rx) = mpsc::channel(1);
    let (position, set_position) = signal(shakmaty::Chess::default());
    let selected_square = RwSignal::new(None::<shakmaty::Square>);
    provide_context(selected_square);
    let (player_role, set_player_role) = signal(None::<PlayerRole>);
    let (grabbed_square, set_grabbed_square) = signal(None::<shakmaty::Square>);

    let legal_move_targets = Signal::derive(move || -> Vec<shakmaty::Square> {
        use shakmaty::Position;
        let Some(selected) = selected_square.get() else {
            return vec![];
        };
        position
            .get()
            .legal_moves()
            .into_iter()
            .filter(|m| m.from() == Some(selected))
            .map(|m| m.to())
            .collect()
    });
    provide_context(legal_move_targets);

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
                                    let chess = position_fen
                                        .parse::<Fen>()
                                        .unwrap()
                                        .into_position::<shakmaty::Chess>(
                                            shakmaty::CastlingMode::Standard,
                                        )
                                        .unwrap();
                                    set_position.set(chess);
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
                    each={move || {
                        match perspective {
                            BoardPerspective::White => (0..8usize).rev()
                                .flat_map(|rank| (0..8usize).map(move |file| shakmaty::Square::new((rank * 8 + file) as u32)))
                                .collect::<Vec<_>>(),
                            BoardPerspective::Black => (0..8usize)
                                .flat_map(|rank| (0..8usize).rev().map(move |file| shakmaty::Square::new((rank * 8 + file) as u32)))
                                .collect::<Vec<_>>(),
                        }
                    }}
                    key=|sq: &shakmaty::Square| *sq as u8
                    let(sq)
                >
                    {
                        let rank = sq.rank().to_usize();
                        let file = sq.file().to_usize();

                        let piece = move || { use shakmaty::Position; position.get().board().piece_at(sq) };

                        view! {
                            <Square rank={rank} file={file} piece={piece()} perspective={perspective} />
                        }
                    }
                </For>
            </div>
        </div>
    }
}
