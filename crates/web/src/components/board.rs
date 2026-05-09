#[cfg(feature = "ssr")]
use crate::state::GameId;
#[cfg(feature = "ssr")]
use crate::state::GameRooms;

use leptos::{prelude::*, server};
use server_fn::{codec::JsonEncoding, BoxedStream, ServerFnError, Websocket};
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
    use axum::extract::Query;
    use futures::StreamExt;
    use leptos_axum::extract;
    use tokio_stream::wrappers::BroadcastStream;

    let mut input = input;

    let Query(game_id) = extract::<Query<GameId>>().await?;
    let games = expect_context::<GameRooms>();
    let game_room = games.lock().await.get(&game_id).cloned();

    if let Some(game_room) = game_room {
        let rx = game_room.lock().await.subscribe();

        let game_room = game_room.clone();
        // spawn a task to listen to the input stream of messages coming in over the websocket
        tokio::spawn(async move {
            while let Some(msg) = input.next().await {
                if let Ok(msg) = msg {
                    println!("Received message from client: {:?}", msg);
                    let _game_room = game_room.lock().await;
                    match msg {
                        ClientMessage::UserJoined { uuid } => todo!(),
                        ClientMessage::UserLeft { uuid } => todo!(),
                        ClientMessage::MoveMade { uci } => todo!(),
                        ClientMessage::Chat { user, text } => todo!(),
                    }
                }
            }
        });

        let stream = BroadcastStream::new(rx)
            .map(|result| result.map_err(|e| ServerFnError::new(e.to_string())));
        Ok(stream.into())
    } else {
        Err(ServerFnError::new(format!("game {} not found", game_id)))
    }
}

#[component]
pub fn Board() -> impl IntoView {
    view! {
        <div class="flex items-center justify-center w-full h-[calc(100vh-3.5rem)]">
            <div class="grid grid-cols-8 grid-rows-8 w-[min(100vw,calc(100vh-11.5rem))] aspect-square">
                {(0..8).into_iter().map(|row| {
                    (0..8).into_iter().map(move |col| {
                        view! {
                            <div
                                class="w-full h-full"
                                class:bg-white=move || (row + col) % 2 == 0
                                class:bg-gray-800=move || (row + col) % 2 != 0
                            >
                            </div>
                        }
                    }).collect_view()
                }).collect_view()}
            </div>
        </div>
    }
}
