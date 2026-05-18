use leptos::prelude::*;
use shared::PlayerRole;
use uuid::Uuid;

use crate::components::{use_current_user, BoardPerspective, BoardUser, ChessBoard};
use crate::game::get_game_info;

#[component]
pub fn PlayBoard(game_id: Uuid) -> impl IntoView {
    use futures::channel::mpsc;
    use futures::StreamExt;
    use leptos::task::spawn_local;
    use shakmaty::fen::Fen;
    use shared::{GameClientMessage, GameServerMessage};

    use crate::websocket::game_websocket;

    let user = use_current_user();

    let (tx, rx) = mpsc::unbounded::<GameClientMessage>();

    let (position, set_position) = signal(shakmaty::Chess::default());
    let last_move = RwSignal::new(None::<(shakmaty::Square, shakmaty::Square)>);
    let (player_role, set_player_role) = signal(None::<PlayerRole>);

    let perspective = Signal::derive(move || BoardPerspective::from(player_role.get()));

    // on_move: gate by turn ownership, send to server via WS.
    let on_move = {
        let tx = tx.clone();
        Callback::new(move |m: shakmaty::Move| {
            use shakmaty::Position as _;
            let pos = position.get_untracked();
            let my_color = player_role.get_untracked().and_then(|r| r.color());
            if my_color != Some(pos.turn()) {
                leptos::logging::warn!("attempted move out of turn");
                return;
            }
            let uci = m.to_uci(shakmaty::CastlingMode::Standard).to_string();
            let _ = tx.unbounded_send(GameClientMessage::MoveMade { uci });
        })
    };

    // can_drag_piece: only the side this player controls.
    let can_drag_piece = Callback::new(move |p: shakmaty::Piece| {
        player_role
            .get()
            .and_then(|r| r.color())
            .is_some_and(|c| c == p.color)
    });

    let tx_join = tx.clone();

    if cfg!(feature = "hydrate") {
        spawn_local(async move {
            let Some(_my_uuid) = user.await.ok().flatten().map(|u| u.id) else {
                leptos::logging::warn!("PlayBoard mounted without authenticated user");
                return;
            };

            let _ = tx_join.unbounded_send(GameClientMessage::UserJoined { game_id });

            match game_websocket(rx.map(Ok).into()).await {
                Ok(mut messages) => {
                    while let Some(msg) = messages.next().await {
                        let Ok(msg) = msg else { continue };
                        match msg {
                            GameServerMessage::UserJoined {
                                uuid,
                                position_fen,
                                player_role: role,
                            } => {
                                if let Ok(fen) = position_fen.parse::<Fen>() {
                                    if let Ok(chess) = fen
                                        .into_position::<shakmaty::Chess>(
                                            shakmaty::CastlingMode::Standard,
                                        )
                                    {
                                        set_position.set(chess);
                                    }
                                }
                                if Some(uuid) == user.await.ok().flatten().map(|u| u.id) {
                                    set_player_role.set(Some(role));
                                }
                            }
                            GameServerMessage::UserLeft { username: _ } => {}
                            GameServerMessage::MoveMade { uci } => {
                                use shakmaty::{uci::UciMove, Position as _};
                                if let Ok(uci_move) = uci.parse::<UciMove>() {
                                    if let Ok(m) =
                                        uci_move.to_move(&position.get_untracked())
                                    {
                                        if let Some(from) = m.from() {
                                            last_move.set(Some((from, m.to())));
                                        }
                                        set_position.update(|pos| {
                                            if let Ok(new_pos) = pos.clone().play(m) {
                                                *pos = new_pos;
                                            }
                                        });
                                    }
                                }
                            }
                            GameServerMessage::Chat { user: _, text: _ } => {}
                        }
                    }
                }
                Err(e) => leptos::logging::warn!("websocket error: {e}"),
            }
        });
    }

    let game_info = Resource::new(move || game_id, |id| async move { get_game_info(id).await });

    view! {
        <div class="flex flex-col items-center w-full h-[calc(100vh-3.5rem)]">
            <Transition fallback=|| view! { <div class="h-12"></div> }>
                {move || game_info.get().and_then(|res| res.ok()).map(|info| {
                    // Top player = the one we're NOT viewing as.
                    let top = match perspective.get() {
                        BoardPerspective::White => info.black.clone(),
                        BoardPerspective::Black => info.white.clone(),
                    };
                    view! { <BoardUser player={top} /> }
                })}
            </Transition>
            <ChessBoard
                position={position}
                perspective={perspective}
                last_move={last_move}
                on_move={on_move}
                can_drag_piece={can_drag_piece}
            />
            <Transition fallback=|| view! { <div class="h-12"></div> }>
                {move || game_info.get().and_then(|res| res.ok()).map(|info| {
                    let bottom = match perspective.get() {
                        BoardPerspective::White => info.white.clone(),
                        BoardPerspective::Black => info.black.clone(),
                    };
                    view! { <BoardUser player={bottom} /> }
                })}
            </Transition>
        </div>
    }
}
