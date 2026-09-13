use leptos::prelude::*;
use shakmaty::{Color, Role, Square};

#[cfg(feature = "hydrate")]
use crate::components::move_target;
use crate::components::{BoardPerspective, PendingPromotion};

/// Overlay shown while a promotion's role (Q/R/B/N) is ambiguous. Anchored
/// to the promoting square with pure CSS percentages — a promotion always
/// lands on rank 1 or 8, which is always the board's top or bottom visual
/// row regardless of perspective, so the stack always has room to grow
/// toward the board's center and never needs pixel geometry.
#[component]
pub fn PromotionPicker(perspective: Signal<BoardPerspective>) -> impl IntoView {
    let pending = expect_context::<RwSignal<Option<PendingPromotion>>>();

    // The card (and its on_click_outside/Escape listeners) is its own
    // component, gated by this Show, so it's only ever constructed once
    // `pending` is `Some` — at which point its `card_ref` div is part of
    // the same view being built and is guaranteed to attach. Registering
    // `on_click_outside` unconditionally at this outer level instead (the
    // bug this replaced) set it up against a `NodeRef` that might never
    // attach, which crashed on mount (`RuntimeError: null function` deep in
    // leptos-use's event-listener setup) every time a game board loaded.
    view! {
        <Show when=move || pending.get().is_some()>
            <PromotionPickerCard perspective=perspective />
        </Show>
    }
}

#[component]
fn PromotionPickerCard(perspective: Signal<BoardPerspective>) -> impl IntoView {
    let pending = expect_context::<RwSignal<Option<PendingPromotion>>>();
    let position = expect_context::<Signal<shakmaty::Chess>>();
    #[cfg(feature = "hydrate")]
    let on_move = expect_context::<Callback<shakmaty::Move>>();

    let card_ref = NodeRef::<leptos::html::Div>::new();

    #[cfg(feature = "hydrate")]
    {
        use leptos::ev;
        let stop = leptos_use::on_click_outside(card_ref, move |_| pending.set(None));
        on_cleanup(stop);
        window_event_listener(ev::keydown, move |e: web_sys::KeyboardEvent| {
            if e.key() == "Escape" && pending.get_untracked().is_some() {
                pending.set(None);
            }
        });
    }

    let choose = move |role: Role| {
        #[cfg(feature = "hydrate")]
        {
            use shakmaty::Position as _;
            if let Some(p) = pending.get_untracked() {
                let legal = position.get_untracked().legal_moves();
                if let Some(m) = legal
                    .iter()
                    .find(|m| m.from() == Some(p.from) && move_target(m) == p.to && m.promotion() == Some(role))
                {
                    on_move.run(*m);
                }
            }
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = role;
        pending.set(None);
    };

    const ROLES: [(Role, &str); 4] = [
        (Role::Queen, "Q"),
        (Role::Knight, "N"),
        (Role::Rook, "R"),
        (Role::Bishop, "B"),
    ];

    view! {
        {move || {
            use shakmaty::Position as _;
            // `Show` above guarantees `pending` is `Some` for the entire
            // lifetime of this component.
            let Some(p) = pending.get() else { return ().into_any() };
            let (row, col) = visual_row_col(p.to, perspective.get());
            let color = position.get().turn();
            let color_char = if color == Color::White { "w" } else { "b" };
            let anchor_style = format!(
                "left: calc({col} * 12.5%); width: 12.5%; {}",
                if row == 0 { "top: 0;" } else { "bottom: 0;" }
            );

            view! {
                <div
                    node_ref=card_ref
                    class="absolute z-40 flex flex-col rounded-md overflow-hidden shadow-xl bg-zinc-900/95 border border-zinc-700"
                    style=anchor_style
                >
                    {ROLES.iter().map(|&(role, role_char)| {
                        let src = format!("/piece/alpha/{color_char}{role_char}.svg");
                        view! {
                            <button
                                class="aspect-square w-full bg-transparent hover:bg-zinc-700/60 cursor-pointer p-0.5"
                                on:click=move |_| choose(role)
                            >
                                <img src=src draggable="false" class="w-full h-full pointer-events-none select-none" />
                            </button>
                        }
                    }).collect_view()}
                </div>
            }.into_any()
        }}
    }
}

/// (visual row, visual col), each 0..8 — matches the same top-left-origin
/// grid ordering `ChessBoard`'s own `<For>` renders squares in.
fn visual_row_col(sq: Square, perspective: BoardPerspective) -> (usize, usize) {
    let (rank, file) = (sq.rank().to_usize(), sq.file().to_usize());
    match perspective {
        BoardPerspective::White => (7 - rank, file),
        BoardPerspective::Black => (rank, 7 - file),
    }
}
