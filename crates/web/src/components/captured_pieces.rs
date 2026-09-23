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
pub fn CapturedPieces(#[prop(into)] position: Signal<shakmaty::Chess>, color: Color) -> impl IntoView {
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

    // Desktop gets noticeably larger pieces and a shallower overlap: at the
    // old flat `w-4` with a `-ml-3` stack, a full set of captures was a
    // barely-legible smudge on a 27" display. `flex-wrap` plus `min-w-0` on
    // the parent means a long capture list now wraps to a second line
    // instead of being clipped by the surrounding `overflow-hidden` cell.
    view! {
        <div class="flex flex-wrap items-center gap-y-0.5 min-h-5 md:min-h-7">
            {move || {
                state
                    .get()
                    .into_iter()
                    .map(|(i, role, first_in_group)| {
                        let piece_class = format!("pc pc-w{}", role.upper_char());
                        let size = "w-4 h-4 md:w-6 md:h-6";
                        let size = format!("{size} {piece_class}");
                        let cls = if i == 0 {
                            format!("{size} flex-shrink-0")
                        } else if first_in_group {
                            format!("{size} flex-shrink-0 ml-0.5 md:ml-1")
                        } else {
                            format!("{size} flex-shrink-0 -ml-3 md:-ml-4")
                        };
                        view! { <div class=cls /> }
                    })
                    .collect_view()
            }}
        </div>
    }
}
