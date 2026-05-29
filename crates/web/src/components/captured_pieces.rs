use leptos::prelude::*;
use shakmaty::{ByRole, Color, Role};

/// Captured-material advantage for `color`: positive means `color` has captured
/// more material value than its opponent.
pub fn material_advantage(pos: &shakmaty::Chess, color: Color) -> i32 {
    use shakmaty::Position as _;

    fn score(cap: &ByRole<u8>) -> i32 {
        cap.pawn as i32
            + cap.knight as i32 * 3
            + cap.bishop as i32 * 3
            + cap.rook as i32 * 5
            + cap.queen as i32 * 9
    }

    let board = pos.board();
    let lost = |c: Color| {
        let m = board.material_side(c);
        ByRole {
            pawn: 8u8.saturating_sub(m.pawn),
            knight: 2u8.saturating_sub(m.knight),
            bishop: 2u8.saturating_sub(m.bishop),
            rook: 2u8.saturating_sub(m.rook),
            queen: 1u8.saturating_sub(m.queen),
            king: 0u8,
        }
    };

    score(&lost(color.other())) - score(&lost(color))
}

#[component]
pub fn CapturedPieces(position: ReadSignal<shakmaty::Chess>, color: Color) -> impl IntoView {
    use shakmaty::Position as _;

    // Flat list: (index, role, is_first_of_its_type). Keyed by index for correct diffing.
    let state = Signal::derive(move || {
        let pos = position.get();
        let board = pos.board();
        let m = board.material_side(color);
        let counts = [
            (Role::Pawn, 8u8.saturating_sub(m.pawn)),
            (Role::Knight, 2u8.saturating_sub(m.knight)),
            (Role::Bishop, 2u8.saturating_sub(m.bishop)),
            (Role::Rook, 2u8.saturating_sub(m.rook)),
            (Role::Queen, 1u8.saturating_sub(m.queen)),
        ];
        let mut pieces: Vec<(usize, Role, bool)> = Vec::new();
        for (role, count) in counts {
            for j in 0..count {
                pieces.push((pieces.len(), role, j == 0));
            }
        }
        pieces
    });

    view! {
        <div class="flex items-center h-5">
            {move || {
                state
                    .get()
                    .into_iter()
                    .map(|(i, role, first_in_group)| {
                        let src = format!("/piece/alpha/w{}.svg", role.upper_char());
                        let cls = if i == 0 {
                            "w-4 h-4"
                        } else if first_in_group {
                            "w-4 h-4 ml-0.5"
                        } else {
                            "w-4 h-4 -ml-3"
                        };
                        view! { <img src=src class=cls draggable="false" /> }
                    })
                    .collect_view()
            }}
        </div>
    }
}
