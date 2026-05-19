use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use shakmaty::attacks::attacks;
use shakmaty::{Bitboard, File};
use shared::{PlayerRole, Side};

use crate::components::Square;

/// Square the user visually drops on for a given move.
/// For castling, shakmaty's `.to()` returns the rook square; this returns the
/// king's destination (g/c file) instead. Use this everywhere we compare a
/// drop/click target to a legal move.
pub fn move_target(m: &shakmaty::Move) -> shakmaty::Square {
    match m {
        shakmaty::Move::Castle { king, rook } => {
            let dest_file = if rook.file() > king.file() {
                File::G
            } else {
                File::C
            };
            shakmaty::Square::from_coords(dest_file, king.rank())
        }
        other => other.to(),
    }
}

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
        // For our piece + our turn: just legal moves.
        // For our piece + opponent's turn: legal-as-if-our-turn (for premoves +
        // castling) UNION pseudo-attacks (threat-analysis view).
        let view_pos = if piece.color == pos.turn() {
            Some(pos.clone())
        } else {
            // swap_turn fails if swapping leaves the new side-to-move in check.
            pos.clone().swap_turn().ok()
        };

        let mut targets: std::collections::HashSet<shakmaty::Square> = view_pos
            .into_iter()
            .flat_map(|p| {
                p.legal_moves()
                    .into_iter()
                    .filter(|m| m.from() == Some(selected))
                    .map(|m| move_target(&m))
                    .collect::<Vec<_>>()
            })
            .collect();

        // Add pseudo-attacks when it's not our turn so the user can also see
        // potential threat rays.
        if piece.color != pos.turn() {
            targets.extend(pseudo_attacks(selected, piece));
        }

        targets.into_iter().collect()
    });

    fn pseudo_attacks(selected: shakmaty::Square, piece: shakmaty::Piece) -> Vec<shakmaty::Square> {
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
                            targets.push(shakmaty::Square::new(((rank - 2) * 8 + file) as u32));
                        }
                    }
                }
            }
        }
        targets
    }

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
