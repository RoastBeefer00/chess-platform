use leptos::{html::Img, prelude::*};
use shakmaty::{Color, Piece, Square};

use crate::components::BoardPerspective;
#[cfg(feature = "hydrate")]
use crate::components::{chess_board::matching_moves, DragState, PendingPromotion};

#[component]
pub fn Square(
    rank: usize,
    file: usize,
    piece: Signal<Option<Piece>>,
    perspective: Signal<BoardPerspective>,
) -> impl IntoView {
    let valid_move_targets = expect_context::<Signal<Vec<shakmaty::Square>>>();
    let selected_square = expect_context::<RwSignal<Option<shakmaty::Square>>>();
    let last_move = expect_context::<RwSignal<Option<(shakmaty::Square, shakmaty::Square)>>>();
    let position = expect_context::<Signal<shakmaty::Chess>>();
    #[cfg(feature = "hydrate")]
    let on_move = expect_context::<Callback<shakmaty::Move>>();
    #[cfg(feature = "hydrate")]
    let on_premove = expect_context::<Callback<(shakmaty::Square, shakmaty::Square)>>();
    let premoves_ctx = use_context::<RwSignal<Vec<(shakmaty::Square, shakmaty::Square)>>>();

    #[cfg(feature = "hydrate")]
    let is_my_turn = expect_context::<Signal<bool>>();
    #[cfg(feature = "hydrate")]
    let pending_promotion = expect_context::<RwSignal<Option<PendingPromotion>>>();

    let in_check = Signal::derive(move || {
        use shakmaty::Position as _;
        let Some(p) = piece.get() else { return false };
        if p.role != shakmaty::Role::King {
            return false;
        }
        let pos = position.get();
        p.color == pos.turn() && pos.is_check()
    });

    let image_path = Signal::derive(move || {
        piece.get().map(|p| {
            let color = match p.color {
                Color::White => "w",
                Color::Black => "b",
            };
            let role = match p.role {
                shakmaty::Role::Pawn => "P",
                shakmaty::Role::Knight => "N",
                shakmaty::Role::Bishop => "B",
                shakmaty::Role::Rook => "R",
                shakmaty::Role::Queen => "Q",
                shakmaty::Role::King => "K",
            };
            format!("/piece/alpha/{}{}.svg", color, role)
        })
    });

    fn rank_to_char(rank: usize) -> char {
        std::char::from_digit((rank + 1) as u32, 10).unwrap()
    }

    fn file_to_char(file: usize) -> char {
        (b'a' + file as u8) as char
    }

    #[cfg(feature = "hydrate")]
    let on_click = move |_: leptos::ev::MouseEvent| {
        let this_square = Square::new((rank * 8 + file) as u32);
        let Some(from_sq) = selected_square.get_untracked() else {
            return;
        };
        if !valid_move_targets.get_untracked().contains(&this_square) {
            return;
        }
        use shakmaty::Position as _;
        if is_my_turn.get_untracked() {
            let legal = position.get_untracked().legal_moves();
            match matching_moves(&legal, from_sq, this_square).as_slice() {
                [] => {}
                [m] => {
                    selected_square.set(None);
                    on_move.run(*m);
                }
                _ => {
                    selected_square.set(None);
                    pending_promotion.set(Some(PendingPromotion {
                        from: from_sq,
                        to: this_square,
                    }));
                }
            }
        } else {
            selected_square.set(None);
            on_premove.run((from_sq, this_square));
        }
    };
    #[cfg(not(feature = "hydrate"))]
    let on_click = |_: leptos::ev::MouseEvent| {};

    let this_sq = Square::new((rank * 8 + file) as u32);
    let is_highlighted = move || {
        selected_square.get() == Some(this_sq)
            || last_move
                .get()
                .is_some_and(|(f, t)| f == this_sq || t == this_sq)
    };
    let is_premove_square = Signal::derive(move || {
        let Some(p) = premoves_ctx else { return false };
        p.get().iter().any(|(f, t)| *f == this_sq || *t == this_sq)
    });

    view! {
        <div
            class="relative w-full h-full select-none touch-none"
            class:bg-white=move || !(rank + file).is_multiple_of(2) && !is_highlighted() && !is_premove_square.get()
            class:bg-green-800=move || (rank + file).is_multiple_of(2) && !is_highlighted() && !is_premove_square.get()
            class:bg-green-300=move || !(rank + file).is_multiple_of(2) && is_highlighted() && !is_premove_square.get()
            class:bg-green-600=move || (rank + file).is_multiple_of(2) && is_highlighted() && !is_premove_square.get()
            class:bg-gray-300=move || !(rank + file).is_multiple_of(2) && is_premove_square.get()
            class:bg-gray-500=move || (rank + file).is_multiple_of(2) && is_premove_square.get()
            data-square=format!("{}{}", file_to_char(file), rank_to_char(rank))
            on:click=on_click
        >
            <Show when=move || in_check.get()>
                <div class="absolute inset-0 bg-red-500 pointer-events-none animate-pulse"></div>
            </Show>
            <Show
                when=move || valid_move_targets.get().contains(&Square::new((rank * 8 + file) as u32))
            >
                {move || if piece.get().is_some() {
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
            <Show when=move || image_path.get().is_some()>
                <DraggablePieceImg rank=rank file=file piece=piece image_path=image_path />
            </Show>
            <Show when=move || perspective.get() == BoardPerspective::White && rank == 0 || perspective.get() == BoardPerspective::Black && rank == 7>
                <span
                    class="absolute bottom-0 left-0.5 font-bold text-sm"
                    class:text-white=move || (rank + file).is_multiple_of(2)
                    class:text-green-800=move || !(rank + file).is_multiple_of(2)
                >{file_to_char(file)}</span>
            </Show>
            <Show when=move || perspective.get() == BoardPerspective::White && file == 7 || perspective.get() == BoardPerspective::Black && file == 0>
                <span
                    class="absolute top-0 right-0.5 font-bold text-sm"
                    class:text-white=move || (rank + file).is_multiple_of(2)
                    class:text-green-800=move || !(rank + file).is_multiple_of(2)
                >{rank_to_char(rank)}</span>
            </Show>
        </div>
    }
}

/// The draggable piece `<img>` for one square. Drag tracking itself lives at
/// the `ChessBoard` level via the shared `DragState` context — this
/// component only starts a drag (`on:pointerdown`, a plain Leptos-delegated
/// event, not a raw per-element listener) and renders the floating image
/// while `drag_state` names this square as the one being dragged.
///
/// This replaced a per-square `leptos_use::use_draggable_with_options` call
/// (up to 32 simultaneous instances, one per occupied square). That hook's
/// `pointermove`/`pointerup` listeners default to `window` and are
/// deliberately leaked forever by `leptos-use` (`Closure::into_js_value`,
/// cleanup only removes the DOM listener). With that many concurrently-leaked
/// window listeners the WASM runtime reproducibly crashed a few seconds
/// after any board with pieces loaded (`RuntimeError: null function`, deep in
/// wasm-bindgen's closure dispatch) — independent of whether anything was
/// actually being dragged. A single pair of board-level listeners (see
/// `ChessBoard`) sidesteps the whole class of problem.
#[component]
fn DraggablePieceImg(
    rank: usize,
    file: usize,
    piece: Signal<Option<Piece>>,
    image_path: Signal<Option<String>>,
) -> impl IntoView {
    #[cfg(feature = "hydrate")]
    let selected_square = expect_context::<RwSignal<Option<shakmaty::Square>>>();
    #[cfg(feature = "hydrate")]
    let drag_state = expect_context::<RwSignal<Option<DragState>>>();
    #[cfg(feature = "hydrate")]
    let can_drag_piece = expect_context::<Callback<shakmaty::Piece, bool>>();
    #[cfg(feature = "hydrate")]
    let valid_move_targets = expect_context::<Signal<Vec<shakmaty::Square>>>();

    #[cfg(not(feature = "hydrate"))]
    let _ = (rank, file, piece);

    let el = NodeRef::<Img>::new();
    #[cfg(feature = "hydrate")]
    let this_sq = Square::new((rank * 8 + file) as u32);

    #[cfg(feature = "hydrate")]
    let on_pointer_down = move |ev: leptos::ev::PointerEvent| {
        // Right-click (button 2) starts an arrow-drag instead (see
        // `ChessBoard`'s board-level `on:pointerdown`) — most squares a user
        // right-clicks are occupied, so without this guard a right-click on
        // a piece would also start moving/selecting it.
        if ev.button() != 0 {
            return;
        }
        // A piece is already selected and this square is one of its legal
        // targets — i.e. this is a capture, clicked rather than dragged.
        // `Square`'s own `on:click` already completes exactly this move;
        // starting a fresh drag/selection on the piece being captured
        // instead (the default below) would hijack it, showing *that*
        // piece's own moves rather than completing the capture — the bug
        // this guards against. Only ever true for an enemy-occupied
        // square: nothing here since a legal move can never target a
        // square one of the mover's own pieces occupies.
        if selected_square.get_untracked().is_some() && valid_move_targets.get_untracked().contains(&this_sq) {
            return;
        }
        let Some(p) = piece.get_untracked() else {
            return;
        };
        if !can_drag_piece.run(p) {
            return;
        }
        let Some(element) = el.get_untracked() else {
            return;
        };
        let rect = element.get_bounding_client_rect();
        let grab_dx = ev.client_x() as f64 - rect.left();
        let grab_dy = ev.client_y() as f64 - rect.top();

        selected_square.set(Some(this_sq));
        drag_state.set(Some(DragState {
            from: this_sq,
            width: rect.width(),
            height: rect.height(),
            grab_dx,
            grab_dy,
            x: rect.left(),
            y: rect.top(),
        }));
    };
    #[cfg(not(feature = "hydrate"))]
    let on_pointer_down = |_: leptos::ev::PointerEvent| {};

    #[cfg(feature = "hydrate")]
    let style = move || match drag_state.get() {
        Some(d) if d.from == this_sq => format!(
            "position: fixed; left: {}px; top: {}px; pointer-events: none; width: {}px; height: {}px; z-index: 50;",
            d.x, d.y, d.width, d.height
        ),
        _ => String::new(),
    };
    #[cfg(not(feature = "hydrate"))]
    let style = || String::new();

    #[cfg(feature = "hydrate")]
    let is_dragging = move || drag_state.get().is_some_and(|d| d.from == this_sq);
    #[cfg(not(feature = "hydrate"))]
    let is_dragging = || false;

    view! {
        <img
            src=move || image_path.get().unwrap_or_default()
            node_ref=el
            draggable="false"
            class="relative z-10 w-full h-full cursor-grab select-none touch-none"
            class:cursor-grabbing=is_dragging
            style=style
            on:pointerdown=on_pointer_down
        />
    }
}
