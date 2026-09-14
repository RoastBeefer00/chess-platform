use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use shakmaty::attacks::attacks;
use shakmaty::{Bitboard, File};
use shared::{PlayerRole, Side};

use crate::components::{PromotionPicker, Square};

/// A move whose destination is ambiguous only in promotion role (Q/R/B/N) —
/// the picker is showing while this is `Some`, and no move has been played
/// yet.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PendingPromotion {
    pub from: shakmaty::Square,
    pub to: shakmaty::Square,
}

/// Live state of an in-progress piece drag, shared via context and owned by
/// `ChessBoard` rather than per-square. Earlier this tracked drag state via
/// `leptos_use::use_draggable_with_options` called once per occupied square
/// (up to 32 simultaneous instances) — each registering its own permanent
/// (`leptos-use` deliberately leaks these via `into_js_value`) window-level
/// `pointermove`/`pointerup` listener. That reproducibly crashed the WASM
/// runtime (`RuntimeError: null function`) a few seconds after any board
/// with pieces loaded. A single board-level `pointermove`/`pointerup` pair
/// (below) plus a plain `on:pointerdown` per piece (Leptos's own delegated
/// event system, not a raw per-element listener) avoids the whole class of
/// problem and is simpler besides.
#[derive(Debug, Clone, Copy)]
pub struct DragState {
    pub from: shakmaty::Square,
    pub width: f64,
    pub height: f64,
    /// Offset from the dragged image's top-left corner to where the pointer
    /// grabbed it — preserved through the drag so the image doesn't jump to
    /// re-center under the cursor.
    pub grab_dx: f64,
    pub grab_dy: f64,
    /// Current floating position (viewport-fixed) of the dragged image.
    pub x: f64,
    pub y: f64,
}

/// Live state of an in-progress right-click-drag annotation arrow. `to` is
/// the square currently under the cursor (for the live preview), re-hit-
/// tested on every `pointermove` — `None` once the cursor leaves the board.
#[derive(Debug, Clone, Copy)]
struct ArrowDrag {
    from: shakmaty::Square,
    to: Option<shakmaty::Square>,
}

/// Every legal move from `from` to `to`. In standard chess this is at most
/// one move — except a promotion, where it's the four role choices
/// (Q/R/B/N), which is exactly the ambiguity the promotion picker resolves.
#[cfg(feature = "hydrate")]
pub(crate) fn matching_moves(legal: &[shakmaty::Move], from: shakmaty::Square, to: shakmaty::Square) -> Vec<shakmaty::Move> {
    legal
        .iter()
        .filter(|m| m.from() == Some(from) && move_target(m) == to)
        .copied()
        .collect()
}

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

/// A square's center, in `0..8` board-unit coordinates matching the arrow
/// overlay's `viewBox="0 0 8 8"` — mirrors the `<For>` grid's own
/// perspective-dependent placement below (White: rank 8 at the top, a-file
/// on the left; Black: both mirrored) so an arrow lands on the same visual
/// square as the piece there, flipped or not.
fn square_center(sq: shakmaty::Square, perspective: BoardPerspective) -> (f64, f64) {
    let file = sq.file().to_usize() as f64;
    let rank = sq.rank().to_usize() as f64;
    let (col, row) = match perspective {
        BoardPerspective::White => (file, 7.0 - rank),
        BoardPerspective::Black => (7.0 - file, rank),
    };
    (col + 0.5, row + 0.5)
}

/// The points of a best-move arrow from `from` to `to` (both square
/// centers, in the same `0..8` coordinates as `square_center`). Knight
/// moves — `(|dfile|, |drank|)` of `(1, 2)` or `(2, 1)` — get an L-shaped
/// dogleg (2 units along the move's longer axis, then 1 along the other)
/// instead of a straight line cutting across unrelated squares, matching
/// the convention lichess/chess.com use.
/// `markerWidth`/`markerHeight` of `#best-move-arrowhead` below, in
/// stroke-width units (the marker's `markerUnits` default) — shared with
/// `pull_back_last_point`'s call site so the line-shortening distance
/// always matches however big the marker itself actually renders.
const ARROWHEAD_MARKER_SIZE: f64 = 4.0;

fn arrow_points(from: (f64, f64), to: (f64, f64)) -> Vec<(f64, f64)> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let is_knight_move = ((dx.abs() - 1.0).abs() < 0.01 && (dy.abs() - 2.0).abs() < 0.01)
        || ((dx.abs() - 2.0).abs() < 0.01 && (dy.abs() - 1.0).abs() < 0.01);
    if !is_knight_move {
        return vec![from, to];
    }
    let bend = if dx.abs() > dy.abs() {
        (from.0 + dx.signum() * 2.0, from.1)
    } else {
        (from.0, from.1 + dy.signum() * 2.0)
    };
    vec![from, bend, to]
}

/// Trims the last point back along the final segment's direction by
/// `distance`, so the polyline's own stroke (and its round cap/joins) ends
/// underneath the arrowhead marker's wide base rather than reaching all
/// the way to the tip — otherwise the line's own width pokes out past the
/// marker's narrowing point, visible as the shaft "poking through" the
/// head. No-ops (keeps the segment full-length) if it's shorter than
/// `distance`, which never happens in practice — the shortest possible
/// chess move is one full square (`1.0` board units) and `distance` here
/// is well under that even for the thickest/most prominent arrow.
fn pull_back_last_point(mut points: Vec<(f64, f64)>, distance: f64) -> Vec<(f64, f64)> {
    let n = points.len();
    let (px, py) = points[n - 2];
    let (qx, qy) = points[n - 1];
    let (dx, dy) = (qx - px, qy - py);
    let len = dx.hypot(dy);
    if len > distance {
        let t = (len - distance) / len;
        points[n - 1] = (px + dx * t, py + dy * t);
    }
    points
}

/// One arrow (`from` → `to`), shared by the engine-suggestion and
/// user-drawn-annotation overlays — see `pull_back_last_point` for why the
/// line is trimmed back by the arrowhead's own length before rendering.
/// `#best-move-arrowhead` (defined once in `ChessBoard`'s view) is reused
/// by every arrow regardless of which kind drew it.
fn arrow_polyline(
    from: shakmaty::Square,
    to: shakmaty::Square,
    perspective: BoardPerspective,
    stroke: &str,
    opacity: f64,
    width: f64,
) -> impl IntoView {
    let points = pull_back_last_point(
        arrow_points(square_center(from, perspective), square_center(to, perspective)),
        ARROWHEAD_MARKER_SIZE * width,
    );
    let points_attr = points.iter().map(|(x, y)| format!("{x},{y}")).collect::<Vec<_>>().join(" ");
    view! {
        <polyline
            points=points_attr
            fill="none"
            stroke=stroke.to_string()
            stroke-width=width.to_string()
            stroke-opacity=opacity.to_string()
            stroke-linecap="round"
            stroke-linejoin="round"
            marker-end="url(#best-move-arrowhead)"
        ></polyline>
    }
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
    #[prop(into)] position: Signal<shakmaty::Chess>,
    perspective: Signal<BoardPerspective>,
    last_move: RwSignal<Option<(shakmaty::Square, shakmaty::Square)>>,
    #[prop(into)] on_move: Callback<shakmaty::Move>,
    #[prop(into)] on_premove: Callback<(shakmaty::Square, shakmaty::Square)>,
    #[prop(into)] can_drag_piece: Callback<shakmaty::Piece, bool>,
    /// Whether it is the local user's turn to move the piece currently
    /// selected (i.e. whether a completed drag/click should call `on_move`
    /// rather than `on_premove`). `PlayBoard` derives this from `PlayerRole`
    /// vs. the side to move; an analysis board that lets one user control
    /// both colors can just pass a constant `true`.
    #[prop(into)] is_my_turn: Signal<bool>,
    /// Best-move arrows to draw over the board — `(from, to, rank)`, `rank`
    /// 0 = most prominent. Only `AnalysisBoard` passes these (top-3 engine
    /// candidates); defaults to empty so `PlayBoard`'s call site needs no
    /// changes.
    #[prop(optional, into)] arrows: Signal<Vec<(shakmaty::Square, shakmaty::Square, usize)>>,
    /// Squares to briefly flash red — e.g. `PuzzlesPage` marking an
    /// incorrect attempt. Defaults to empty so other call sites need no
    /// changes; the caller is responsible for clearing it after a beat.
    #[prop(optional, into)] wrong_squares: Signal<Vec<shakmaty::Square>>,
    /// Squares to ring in yellow — `PuzzlesPage`'s hint button marking the
    /// origin square of the correct next move. Defaults to empty so other
    /// call sites need no changes.
    #[prop(optional, into)] hint_squares: Signal<Vec<shakmaty::Square>>,
    /// Overrides the board's sizing classes — e.g. a small fixed size for a
    /// grid tile. Defaults to the full interactive-board sizing so every
    /// existing call site is unaffected.
    #[prop(optional)] size_class: Option<&'static str>,
) -> impl IntoView {
    let board_class = format!(
        "relative grid grid-cols-8 grid-rows-8 {}",
        size_class.unwrap_or("w-[min(100vw,calc(100dvh-11.5rem))] aspect-square")
    );
    let selected_square = RwSignal::new(None::<shakmaty::Square>);
    let pending_promotion = RwSignal::new(None::<PendingPromotion>);
    let drag_state = RwSignal::new(None::<DragState>);
    // Right-click-drag annotation arrows — local to this board (not a prop
    // like `arrows` above), so both `PlayBoard` and `AnalysisBoard` get the
    // feature automatically with no changes of their own.
    let arrow_drag = RwSignal::new(None::<ArrowDrag>);
    let user_arrows = RwSignal::new(Vec::<(shakmaty::Square, shakmaty::Square)>::new());

    // Shapes are per-position, not carried across moves or navigation —
    // matches lichess. `user_arrows` starts empty, so the first (mount) run
    // is a no-op.
    Effect::new(move || {
        position.get();
        user_arrows.set(vec![]);
    });

    let premoves_ctx = use_context::<RwSignal<Vec<(shakmaty::Square, shakmaty::Square)>>>();

    let legal_move_targets = Signal::derive(move || -> Vec<shakmaty::Square> {
        use shakmaty::Position as _;
        let Some(selected) = selected_square.get() else {
            return vec![];
        };
        let pos = position.get();

        // Look up piece in the virtual board (premoves may have moved it to `selected`)
        let queue = premoves_ctx.map(|p| p.get()).unwrap_or_default();
        let piece = if queue.is_empty() {
            pos.board().piece_at(selected)
        } else {
            apply_premoves(pos.board(), &queue).get(&selected).copied()
        };
        let Some(piece) = piece else {
            return vec![];
        };

        // Piece was premoved to `selected` (not in actual board there) — use pseudo-attacks
        // so the user can chain further premoves for that piece.
        if pos.board().piece_at(selected) != Some(piece) {
            return pseudo_attacks(selected, piece);
        }

        // Normal case: piece is at its actual position.
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

    fn apply_premoves(
        board: &shakmaty::Board,
        queue: &[(shakmaty::Square, shakmaty::Square)],
    ) -> std::collections::HashMap<shakmaty::Square, shakmaty::Piece> {
        use shakmaty::Role;
        let mut pieces: std::collections::HashMap<shakmaty::Square, shakmaty::Piece> = board
            .occupied()
            .into_iter()
            .filter_map(|s| board.piece_at(s).map(|p| (s, p)))
            .collect();
        for (from, to) in queue {
            if let Some(mut p) = pieces.remove(from) {
                if p.role == Role::Pawn {
                    let back_rank = if p.color == shakmaty::Color::White { 7 } else { 0 };
                    if to.rank().to_usize() == back_rank {
                        p.role = Role::Queen;
                    }
                }
                pieces.insert(*to, p);
            }
        }
        pieces
    }

    fn pseudo_attacks(selected: shakmaty::Square, piece: shakmaty::Piece) -> Vec<shakmaty::Square> {
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
    provide_context(on_premove);
    provide_context(can_drag_piece);
    provide_context(is_my_turn);
    provide_context(pending_promotion);
    provide_context(drag_state);

    // Single board-level drag tracker, replacing the per-square
    // `use_draggable_with_options` hooks (see `DragState`'s doc comment for
    // why). `pointerdown` stays on each piece's `<img>` via a plain Leptos
    // `on:` binding (delegated, not a raw per-element listener) so it still
    // knows which square/piece the drag started from.
    #[cfg(feature = "hydrate")]
    {
        use leptos::prelude::window_event_listener_untyped;
        use shakmaty::Position as _;
        use std::str::FromStr;
        use wasm_bindgen::JsCast as _;

        // Shared by the left-drag-drop logic below and the right-click
        // arrow-drag logic — resolves a viewport point to the `data-square`
        // element under it. Needed (rather than just using the event's own
        // `target()`) specifically at pointerUP: mid-drag, pointer capture
        // means the event's target is wherever the drag *started*, not
        // whatever's actually under the cursor now.
        fn square_at_point(x: f32, y: f32) -> Option<shakmaty::Square> {
            let window = web_sys::window()?;
            let el = window.document()?.element_from_point(x, y)?;
            let sq_el = el.closest("[data-square]").ok()??;
            shakmaty::Square::from_str(&sq_el.get_attribute("data-square")?).ok()
        }

        // `window_event_listener`'s typed `ev::pointermove`/`ev::pointerup`
        // descriptors cast the raw event to `web_sys::PointerEvent`
        // internally; under rapid repeated firing (e.g. quickly stepping
        // through many moves) that cast reproducibly crashed the WASM
        // runtime (`RuntimeError: null function`, deep in web-sys's
        // generated `PointerEvent` glue) even with no drag in progress —
        // every pointerup anywhere in the window runs this handler. Casting
        // to `MouseEvent` instead (same `client_x`/`client_y` we actually
        // use; every `PointerEvent` genuinely is one in the DOM) goes
        // through different web-sys glue and doesn't hit it.
        window_event_listener_untyped("pointermove", move |e: web_sys::Event| {
            let e: web_sys::MouseEvent = e.unchecked_into();
            drag_state.update(|d| {
                if let Some(d) = d {
                    d.x = e.client_x() as f64 - d.grab_dx;
                    d.y = e.client_y() as f64 - d.grab_dy;
                }
            });
            if arrow_drag.get_untracked().is_some() {
                let sq = square_at_point(e.client_x() as f32, e.client_y() as f32);
                arrow_drag.update(|a| {
                    if let Some(a) = a {
                        a.to = sq;
                    }
                });
            }
        });

        window_event_listener_untyped("pointerup", move |e: web_sys::Event| {
            let e: web_sys::MouseEvent = e.unchecked_into();

            // Complete a right-click arrow drag, if one was in progress —
            // independent of (and unconditional on) any left-drag below.
            if let Some(drag) = arrow_drag.get_untracked() {
                arrow_drag.set(None);
                match drag.to {
                    Some(to) if to != drag.from => user_arrows.update(|arrows| {
                        if let Some(i) = arrows.iter().position(|a| *a == (drag.from, to)) {
                            arrows.remove(i);
                        } else {
                            arrows.push((drag.from, to));
                        }
                    }),
                    // Plain right-click (released on the start square) or
                    // the cursor left the board entirely — clear all.
                    _ => user_arrows.set(vec![]),
                }
            }

            let Some(drag) = drag_state.get_untracked() else {
                return;
            };
            drag_state.set(None);

            // Mobile Safari can produce drops outside the viewport (finger
            // lifted off-screen) — `square_at_point` returning `None` covers
            // that legitimately; bail out instead of panicking the drop
            // handler (which would block all future drags).
            let (x, y) = (e.client_x() as f32, e.client_y() as f32);
            let Some(dropped_square) = square_at_point(x, y) else {
                selected_square.set(None);
                return;
            };

            if !legal_move_targets.get_untracked().contains(&dropped_square) {
                return;
            }

            let from_sq = drag.from;
            if is_my_turn.get_untracked() {
                let legal = position.get_untracked().legal_moves();
                match matching_moves(&legal, from_sq, dropped_square).as_slice() {
                    [] => {}
                    [m] => {
                        selected_square.set(None);
                        on_move.run(*m);
                    }
                    _ => {
                        selected_square.set(None);
                        pending_promotion.set(Some(PendingPromotion {
                            from: from_sq,
                            to: dropped_square,
                        }));
                    }
                }
            } else {
                selected_square.set(None);
                on_premove.run((from_sq, dropped_square));
            }
        });
    }

    // Starts a right-click-drag annotation arrow. A per-element `on:`
    // binding (delegated, like the piece's own `on:pointerdown`) rather than
    // a window listener — no new leak risk. The event's own target is
    // correct here (unlike at pointerup, mid-drag), so this doesn't need
    // `square_at_point`.
    #[cfg(feature = "hydrate")]
    let on_board_pointer_down = move |ev: leptos::ev::PointerEvent| {
        use std::str::FromStr;
        use wasm_bindgen::JsCast as _;
        if ev.button() != 2 {
            return;
        }
        let Some(target) = ev.target() else { return };
        let Ok(el) = target.dyn_into::<web_sys::Element>() else {
            return;
        };
        let Ok(Some(sq_el)) = el.closest("[data-square]") else {
            return;
        };
        let Some(attr) = sq_el.get_attribute("data-square") else {
            return;
        };
        let Ok(sq) = shakmaty::Square::from_str(&attr) else {
            return;
        };
        arrow_drag.set(Some(ArrowDrag { from: sq, to: Some(sq) }));
    };
    #[cfg(not(feature = "hydrate"))]
    let on_board_pointer_down = |_: leptos::ev::PointerEvent| {};

    view! {
        <div class="flex items-center justify-center">
            <div
                class=board_class
                on:pointerdown=on_board_pointer_down
                on:contextmenu=move |e| {
                    e.prevent_default();
                    if selected_square.get_untracked().is_some() {
                        selected_square.set(None);
                    } else if let Some(p) = premoves_ctx {
                        p.set(vec![]);
                    }
                }
            >
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
                            let pos = position.get();
                            let queue = premoves_ctx.map(|p| p.get()).unwrap_or_default();
                            if queue.is_empty() {
                                return pos.board().piece_at(sq);
                            }
                            apply_premoves(pos.board(), &queue).get(&sq).copied()
                        });

                        view! {
                            <Square rank={rank} file={file} piece={piece} perspective={perspective} wrong_squares={wrong_squares} hint_squares={hint_squares} />
                        }
                    }
                </For>
                <svg
                    class="absolute inset-0 pointer-events-none"
                    viewBox="0 0 8 8"
                    style="width: 100%; height: 100%;"
                >
                    <defs>
                        <marker
                            id="best-move-arrowhead"
                            viewBox="0 0 10 10"
                            refX="0"
                            refY="5"
                            markerWidth=ARROWHEAD_MARKER_SIZE.to_string()
                            markerHeight=ARROWHEAD_MARKER_SIZE.to_string()
                            orient="auto-start-reverse"
                        >
                            <path d="M 0 0 L 10 5 L 0 10 z" fill="context-stroke"></path>
                        </marker>
                    </defs>
                    {move || {
                        let persp = perspective.get();
                        arrows
                            .get()
                            .into_iter()
                            .map(|(from, to, rank)| {
                                let (opacity, width) = match rank {
                                    0 => (0.85, 0.18),
                                    1 => (0.55, 0.14),
                                    _ => (0.30, 0.10),
                                };
                                arrow_polyline(from, to, persp, "#9ca3af", opacity, width)
                            })
                            .collect_view()
                    }}
                    // User-drawn (right-click-drag) annotation arrows — a
                    // distinct green so they read as "your annotation" next
                    // to the engine suggestions' gray, drawn on top of them.
                    {move || {
                        let persp = perspective.get();
                        user_arrows
                            .get()
                            .into_iter()
                            .map(|(from, to)| arrow_polyline(from, to, persp, "#22c55e", 0.8, 0.16))
                            .collect_view()
                    }}
                    // Live preview while a right-click-drag is in progress —
                    // same color, faded, so it reads as "not committed yet".
                    // Nothing renders once the cursor leaves the board
                    // (`to: None`) or sits back on the start square (a plain
                    // right-click, not a real drag).
                    {move || {
                        let persp = perspective.get();
                        arrow_drag.get().and_then(|drag| {
                            let to = drag.to?;
                            (to != drag.from).then(|| arrow_polyline(drag.from, to, persp, "#22c55e", 0.4, 0.16))
                        })
                    }}
                </svg>
                <PromotionPicker perspective={perspective} />
            </div>
        </div>
    }
}
