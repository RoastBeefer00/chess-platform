use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use shakmaty::attacks::attacks;
use shakmaty::Bitboard;
use shared::{PlayerRole, Side};

use crate::components::Square;

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
            _ => BoardPerspective::White,
        }
    }
}

#[component]
pub fn ChessBoard(
    position: ReadSignal<shakmaty::Chess>,
    perspective: Signal<BoardPerspective>,
    last_move: RwSignal<Option<(shakmaty::Square, shakmaty::Square)>>,
    #[prop(into)] on_move: Callback<shakmaty::Move>,
    #[prop(into)] can_drag_piece: Callback<shakmaty::Piece, bool>,
) -> impl IntoView {
    let selected_square = RwSignal::new(None::<shakmaty::Square>);

    let legal_move_targets = Signal::derive(move || -> Vec<shakmaty::Square> {
        use shakmaty::Position as _;
        let Some(selected) = selected_square.get() else {
            return vec![];
        };
        let pos = position.get();
        let Some(piece) = pos.board().piece_at(selected) else {
            return vec![];
        };
        // If the selected piece is the side-to-move, show legal moves.
        // Otherwise show pseudo-attacks (visual exploration) — handy for
        // analyzing opponent threats.
        if piece.color == pos.turn() {
            pos.legal_moves()
                .into_iter()
                .filter(|m| m.from() == Some(selected))
                .map(|m| m.to())
                .collect()
        } else {
            use shakmaty::Role;
            let mut targets: Vec<shakmaty::Square> =
                attacks(selected, piece, Bitboard::EMPTY).into_iter().collect();
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
                                targets
                                    .push(shakmaty::Square::new(((rank - 2) * 8 + file) as u32));
                            }
                        }
                    }
                }
            }
            targets
        }
    });

    // Contexts consumed by Square.
    provide_context(position);
    provide_context(selected_square);
    provide_context(last_move);
    provide_context(legal_move_targets);
    provide_context(on_move);
    provide_context(can_drag_piece);

    view! {
        <div class="flex items-center justify-center w-full h-full">
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
                        let piece = Signal::derive(move || {
                            use shakmaty::Position as _;
                            position.get().board().piece_at(sq)
                        });

                        view! {
                            <Square rank={rank} file={file} piece={piece} perspective={perspective} />
                        }
                    }
                </For>
            </div>
        </div>
    }
}
