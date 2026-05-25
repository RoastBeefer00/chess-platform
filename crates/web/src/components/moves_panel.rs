use leptos::prelude::*;
use shakmaty::{san::San, uci::UciMove, Chess, Position};

/// Replay UCI history into SAN strings. Each SAN is rendered against the
/// position *before* the move was played — which is what shakmaty requires.
fn uci_history_to_san(uci_moves: &[String]) -> Vec<String> {
    let mut pos = Chess::default();
    let mut out = Vec::with_capacity(uci_moves.len());
    for uci_str in uci_moves {
        let Ok(uci) = uci_str.parse::<UciMove>() else {
            out.push(uci_str.clone()); // fall back to raw text on bad data
            continue;
        };
        let Ok(mv) = uci.to_move(&pos) else {
            out.push(uci_str.clone());
            continue;
        };
        out.push(San::from_move(&pos, mv).to_string());
        // play() consumes pos and returns the new one; ignore illegal (shouldn't
        // happen if the history is valid — we already proved it parses).
        if let Ok(next) = pos.clone().play(mv) {
            pos = next;
        }
    }
    out
}

#[component]
pub fn MovesPanel(
    moves: ReadSignal<Vec<String>>,
    viewing_ply: ReadSignal<Option<usize>>,
    set_viewing_ply: WriteSignal<Option<usize>>,
    /// Compact horizontal strip layout for mobile. Defaults to the full
    /// vertical panel layout suitable for a desktop side column.
    #[prop(optional)]
    compact: bool,
) -> impl IntoView {
    // SAN list derived from the live UCI history. Reads inside this signal
    // mean any panel cell that reads back into it updates reactively when a
    // new move arrives.
    let san_all = Signal::derive(move || uci_history_to_san(&moves.get()));

    // Move-pair numbers (1..=ceil(N/2)). The `each` re-evaluates when
    // `san_all` grows, so new rows get added as moves come in.
    let move_numbers = Signal::derive(move || {
        let len = san_all.with(Vec::len);
        let pair_count = len.div_ceil(2);
        (1..=pair_count).collect::<Vec<_>>()
    });

    let is_empty = Signal::derive(move || moves.with(Vec::is_empty));

    let selected_move = Signal::derive(move || match viewing_ply.get() {
        Some(i) => i,
        None => moves.get().len(),
    });

    let total = Signal::derive(move || moves.with(Vec::len));
    let can_back = Signal::derive(move || selected_move.get() > 0);
    let can_forward = Signal::derive(move || viewing_ply.get().is_some());

    let go_back = move |_| {
        let cur = selected_move.get_untracked();
        if cur > 0 {
            set_viewing_ply.set(Some(cur - 1));
        }
    };
    let go_live = move |_| set_viewing_ply.set(None);
    let go_beginning = move |_| set_viewing_ply.set(Some(0));
    let go_forward = move |_| {
        let cur = selected_move.get_untracked();
        let max = total.get_untracked();
        let next = cur + 1;
        if next >= max {
            // Moving forward past the latest move = live view.
            set_viewing_ply.set(None);
        } else {
            set_viewing_ply.set(Some(next));
        }
    };

    #[cfg(feature = "hydrate")]
    {
        use leptos::ev;
        use leptos::prelude::window_event_listener;
        use wasm_bindgen::JsCast as _;

        window_event_listener(ev::keydown, move |e: web_sys::KeyboardEvent| {
            // skip when typing in an input
            if let Some(target) = e.target() {
                if let Ok(el) = target.dyn_into::<web_sys::HtmlElement>() {
                    let tag = el.tag_name();
                    if tag == "INPUT" || tag == "TEXTAREA" || el.is_content_editable() {
                        return;
                    }
                }
            }
            match e.key().as_str() {
                "ArrowLeft" => {
                    e.prevent_default();
                    go_back(());
                }
                "ArrowRight" => {
                    e.prevent_default();
                    go_forward(());
                }
                "ArrowUp" => {
                    e.prevent_default();
                    go_live(());
                }
                "ArrowDown" => {
                    e.prevent_default();
                    go_beginning(());
                }
                _ => {}
            }
        });
    }

    if compact {
        compact_view(
            move_numbers,
            san_all,
            selected_move,
            can_back,
            can_forward,
            set_viewing_ply,
            go_beginning,
            go_back,
            go_forward,
            go_live,
        )
        .into_any()
    } else {
        full_view(
            is_empty,
            move_numbers,
            san_all,
            selected_move,
            can_back,
            can_forward,
            set_viewing_ply,
            go_beginning,
            go_back,
            go_forward,
            go_live,
        )
        .into_any()
    }
}

#[allow(clippy::too_many_arguments)]
fn full_view(
    is_empty: Signal<bool>,
    move_numbers: Signal<Vec<usize>>,
    san_all: Signal<Vec<String>>,
    selected_move: Signal<usize>,
    can_back: Signal<bool>,
    can_forward: Signal<bool>,
    set_viewing_ply: WriteSignal<Option<usize>>,
    go_beginning: impl Fn(()) + Copy + Send + Sync + 'static,
    go_back: impl Fn(()) + Copy + Send + Sync + 'static,
    go_forward: impl Fn(()) + Copy + Send + Sync + 'static,
    go_live: impl Fn(()) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let nav_btn_class = "flex-1 px-2 py-1.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded hover:border-zinc-500 hover:text-white transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:border-zinc-700 disabled:hover:text-zinc-300";
    view! {
        <div class="flex flex-col h-full min-h-0 text-sm font-mono">
            <div class="flex-1 min-h-0 overflow-y-auto flex flex-col gap-0.5">
                <Show
                    when=move || !is_empty.get()
                    fallback=|| view! {
                        <div class="text-zinc-500 text-xs italic px-1 py-2">"No moves yet"</div>
                    }
                >
                    <For
                        each=move || move_numbers.get()
                        key=|num| *num
                        children=move |num| {
                            let white = move || san_all.with(|s| {
                                s.get((num - 1) * 2).cloned().unwrap_or_default()
                            });
                            let black = move || san_all.with(|s| {
                                s.get((num - 1) * 2 + 1).cloned()
                            });
                            let white_ply = 2 * num - 1;
                            let black_ply = 2 * num;
                            view! {
                                <div class="flex flex-row items-center gap-2 px-1 py-0.5 rounded hover:bg-zinc-800/50">
                                    <span class="text-zinc-500 w-6 text-right select-none">{num}"."</span>
                                    <button
                                        class="flex-1 text-left px-1 rounded hover:bg-zinc-700 cursor-pointer"
                                        class:bg-zinc-700=move || selected_move.get() == white_ply
                                        on:click=move |_| set_viewing_ply.set(Some(white_ply))
                                    >
                                        {white}
                                    </button>
                                    {move || match black() {
                                        Some(b) => view! {
                                            <button
                                                class="flex-1 text-left px-1 rounded hover:bg-zinc-700 cursor-pointer"
                                                class:bg-zinc-700=move || selected_move.get() == black_ply
                                                on:click=move |_| set_viewing_ply.set(Some(black_ply))
                                            >
                                                {b}
                                            </button>
                                        }.into_any(),
                                        None => view! { <div class="flex-1"></div> }.into_any(),
                                    }}
                                </div>
                            }
                        }
                    />
                </Show>
            </div>
            <div class="flex flex-row items-stretch gap-1 mt-2 pt-2 border-t border-zinc-800">
                <button on:click=move |_| go_beginning(()) disabled=move || !can_back.get() aria-label="Jump to start" class=nav_btn_class>"«"</button>
                <button on:click=move |_| go_back(()) disabled=move || !can_back.get() aria-label="Previous move" class=nav_btn_class>"‹"</button>
                <button on:click=move |_| go_forward(()) disabled=move || !can_forward.get() aria-label="Next move" class=nav_btn_class>"›"</button>
                <button on:click=move |_| go_live(()) disabled=move || !can_forward.get() aria-label="Jump to live" class=nav_btn_class>"»"</button>
            </div>
        </div>
    }
}

#[allow(clippy::too_many_arguments)]
fn compact_view(
    move_numbers: Signal<Vec<usize>>,
    san_all: Signal<Vec<String>>,
    selected_move: Signal<usize>,
    can_back: Signal<bool>,
    can_forward: Signal<bool>,
    set_viewing_ply: WriteSignal<Option<usize>>,
    go_beginning: impl Fn(()) + Copy + Send + Sync + 'static,
    go_back: impl Fn(()) + Copy + Send + Sync + 'static,
    go_forward: impl Fn(()) + Copy + Send + Sync + 'static,
    go_live: impl Fn(()) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let nav_btn_class = "px-1.5 py-1 text-xs font-medium text-zinc-300 border border-zinc-700 rounded hover:border-zinc-500 hover:text-white transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:border-zinc-700 disabled:hover:text-zinc-300 flex-shrink-0";

    // Auto-scroll the strip to follow the live frontier so the latest move
    // is always visible. Skip when the user is in review mode (viewing_ply
    // is Some) so we don't yank them around.
    let strip_ref = NodeRef::<leptos::html::Div>::new();
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        // Re-run whenever the move list grows or the viewer state changes.
        let _ = san_all.with(Vec::len);
        let viewing = selected_move.get();
        let total = san_all.with(Vec::len);
        if let Some(el) = strip_ref.get() {
            // If we're at the live frontier, snap to the end.
            if viewing == total {
                el.set_scroll_left(el.scroll_width());
            }
        }
    });

    view! {
        <div class="flex flex-row items-center gap-1 w-full text-xs font-mono rounded-md bg-zinc-900/60 border border-zinc-800 px-1 py-1">
            <button on:click=move |_| go_beginning(()) disabled=move || !can_back.get() aria-label="Jump to start" class=nav_btn_class>"«"</button>
            <button on:click=move |_| go_back(()) disabled=move || !can_back.get() aria-label="Previous move" class=nav_btn_class>"‹"</button>
            <div
                node_ref=strip_ref
                class="flex-1 min-w-0 overflow-x-auto flex flex-row items-center gap-2 whitespace-nowrap pb-1.5"
            >
                <For
                    each=move || move_numbers.get()
                    key=|num| *num
                    children=move |num| {
                        let white = move || san_all.with(|s| {
                            s.get((num - 1) * 2).cloned().unwrap_or_default()
                        });
                        let black = move || san_all.with(|s| {
                            s.get((num - 1) * 2 + 1).cloned()
                        });
                        let white_ply = 2 * num - 1;
                        let black_ply = 2 * num;
                        view! {
                            <span class="flex flex-row items-center gap-1 flex-shrink-0">
                                <span class="text-zinc-500 select-none">{num}"."</span>
                                <button
                                    class="px-1 rounded hover:bg-zinc-700 cursor-pointer"
                                    class:bg-zinc-700=move || selected_move.get() == white_ply
                                    on:click=move |_| set_viewing_ply.set(Some(white_ply))
                                >
                                    {white}
                                </button>
                                {move || black().map(|b| view! {
                                    <button
                                        class="px-1 rounded hover:bg-zinc-700 cursor-pointer"
                                        class:bg-zinc-700=move || selected_move.get() == black_ply
                                        on:click=move |_| set_viewing_ply.set(Some(black_ply))
                                    >
                                        {b}
                                    </button>
                                })}
                            </span>
                        }
                    }
                />
            </div>
            <button on:click=move |_| go_forward(()) disabled=move || !can_forward.get() aria-label="Next move" class=nav_btn_class>"›"</button>
            <button on:click=move |_| go_live(()) disabled=move || !can_forward.get() aria-label="Jump to live" class=nav_btn_class>"»"</button>
        </div>
    }
}
