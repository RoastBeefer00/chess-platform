use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use shakmaty::attacks::attacks;
use shakmaty::Bitboard;
use shared::{ClientMessage, PlayerRole, ServerMessage, Side};
use uuid::Uuid;

use crate::components::Square;
use crate::websocket::game_websocket;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoardPerspective {
    White,
    Black,
}

impl From<Option<PlayerRole>> for BoardPerspective {
    fn from(role: Option<PlayerRole>) -> Self {
        match role {
            Some(PlayerRole::Player(Side::White)) => BoardPerspective::White,
            Some(PlayerRole::Player(Side::Black)) => BoardPerspective::Black,
            _ => BoardPerspective::White, // default to white if no role
        }
    }
}

#[component]
pub fn Board(game_id: Uuid) -> impl IntoView {
    use futures::channel::mpsc;
    use futures::StreamExt;
    use leptos::task::spawn_local;
    use shakmaty::fen::Fen;

    // Channel to websocket
    let (mut tx, rx) = mpsc::channel(1);
    provide_context(tx.clone());

    // Chess position state
    let (position, set_position) = signal(shakmaty::Chess::default());
    provide_context((position, set_position));

    // Selected square state (for showing legal moves and dragging pieces)
    let selected_square = RwSignal::new(None::<shakmaty::Square>);
    provide_context(selected_square);

    // Last move state (for highlighting from/to squares)
    let last_move = RwSignal::new(None::<(shakmaty::Square, shakmaty::Square)>);
    provide_context(last_move);

    // Player role state (for board perspective and determining if user can move pieces)
    let (player_role, set_player_role) = signal(None::<PlayerRole>);
    provide_context(player_role);

    let legal_move_targets = Signal::derive(move || -> Vec<shakmaty::Square> {
        use shakmaty::Position;
        let Some(selected) = selected_square.get() else {
            return vec![];
        };
        let pos = position.get();
        let Some(piece) = pos.board().piece_at(selected) else {
            return vec![];
        };
        let is_my_turn = player_role
            .get()
            .and_then(|r| r.color())
            .is_some_and(|c| c == pos.turn());
        if is_my_turn {
            leptos::logging::log!("it's my turn");
            pos.legal_moves()
                .into_iter()
                .filter(|m| m.from() == Some(selected))
                .map(|m| m.to())
                .collect()
        } else {
            use shakmaty::Role;
            let mut targets: Vec<shakmaty::Square> = attacks(selected, piece, Bitboard::EMPTY)
                .into_iter()
                .collect();
            if piece.role == Role::Pawn {
                let rank = selected.rank().to_usize();
                let file = selected.file().to_usize();
                match piece.color {
                    shakmaty::Color::White => {
                        targets.push(shakmaty::Square::new(((rank + 1) * 8 + file) as u32));
                        if rank == 1 {
                            targets.push(shakmaty::Square::new(((rank + 2) * 8 + file) as u32));
                        }
                    }
                    shakmaty::Color::Black => {
                        if rank > 0 {
                            targets.push(shakmaty::Square::new(((rank - 1) * 8 + file) as u32));
                            if rank == 6 {
                                targets.push(shakmaty::Square::new(
                                    ((rank - 2) * 8 + file) as u32,
                                ));
                            }
                        }
                    }
                }
            }
            targets
        }
    });
    provide_context(legal_move_targets);

    let perspective = Signal::derive(move || BoardPerspective::from(player_role.get()));

    let my_uuid = Uuid::new_v4();

    if cfg!(feature = "hydrate") {
        spawn_local(async move {
            match game_websocket(rx.map(Ok).into()).await {
                Ok(mut messages) => {
                    while let Some(msg) = messages.next().await {
                        if let Ok(msg) = msg {
                            match msg {
                                ServerMessage::UserJoined {
                                    uuid,
                                    position_fen,
                                    player_role,
                                } => {
                                    let chess = position_fen
                                        .parse::<Fen>()
                                        .unwrap()
                                        .into_position::<shakmaty::Chess>(
                                            shakmaty::CastlingMode::Standard,
                                        )
                                        .unwrap();
                                    set_position.set(chess);
                                    if uuid == my_uuid {
                                        set_player_role.set(Some(player_role));
                                    }
                                }
                                ServerMessage::UserLeft { username: _ } => {}
                                ServerMessage::MoveMade { uci } => {
                                    use shakmaty::{uci::UciMove, Position};
                                    if let Ok(uci_move) = uci.parse::<UciMove>() {
                                        if let Ok(m) = uci_move.to_move(&position.get_untracked()) {
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
                                ServerMessage::Chat { user: _, text: _ } => {}
                            }
                        }
                    }
                }
                Err(e) => leptos::logging::warn!("websocket error: {e}"),
            }
        });
        let _send_result = tx.try_send(ClientMessage::UserJoined {
            uuid: my_uuid,
            game_id,
        });
    }

    view! {
        <div class="flex items-center justify-center w-full h-[calc(100vh-3.5rem)]">
            <div class="grid grid-cols-8 grid-rows-8 w-[min(100vw,calc(100vh-11.5rem))] aspect-square">
                <For
                    each={move || {
                        match perspective.get() {
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

                        let piece = Signal::derive(move || { use shakmaty::Position; position.get().board().piece_at(sq) });


                        view! {
                            <Square rank={rank} file={file} piece={piece} perspective={perspective} />
                        }
                    }
                </For>
            </div>
        </div>
    }
}
