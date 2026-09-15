use leptos::prelude::*;

use super::tree::{DisplayNode, LineItem, MoveLine, MoveTree, NodeId};
use crate::sound;

/// One row in the rendered move list: either a move pair (or, at the start
/// of a variation opening on Black, a lone move) or a nested variation
/// block. Turns strictly alternate within a `LineItem::Moves` run, so
/// grouping on "a White move starts a new row" always yields groups of
/// exactly one or two moves — never more.
enum Row<'a> {
    Pair(&'a [DisplayNode]),
    Nested(&'a MoveLine),
}

fn rows_for(items: &[LineItem]) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    for item in items {
        match item {
            LineItem::Moves(nodes) => {
                let mut i = 0;
                while i < nodes.len() {
                    let end = if nodes[i].is_white && nodes.get(i + 1).is_some_and(|n| !n.is_white) {
                        i + 2
                    } else {
                        i + 1
                    };
                    rows.push(Row::Pair(&nodes[i..end]));
                    i = end;
                }
            }
            LineItem::Variation(sub) => rows.push(Row::Nested(sub)),
        }
    }
    rows
}

/// Renders a [`MoveLine`] as one move pair per row — a real scoresheet
/// layout, not width-dependent text wrapping that can crowd several move
/// pairs onto the same line. A `LineItem::Variation` becomes its own
/// (deeper-indented) nested block of rows, inserted exactly where it
/// branches off; two variations at the same branch point are separate
/// `LineItem::Variation`s, so each gets its own `( )` rather than merging
/// into one. `line_items` (`tree.rs`) guarantees every `MoveLine`'s first
/// and last row is a `Pair`, never a `Nested` block, so the opening/closing
/// parens always have a real move to attach to.
fn render_line<MB>(line: &MoveLine, is_variation: bool, move_button: MB) -> AnyView
where
    MB: Fn(NodeId, String, bool, u32, bool) -> AnyView + Copy + 'static,
{
    let rows = rows_for(&line.items);
    let last = rows.len().saturating_sub(1);
    let indent = format!("padding-left: {}rem", line.depth as f64 * 1.0);

    let children = rows
        .into_iter()
        .enumerate()
        .map(|(i, row)| match row {
            Row::Pair(nodes) => {
                let buttons = nodes
                    .iter()
                    .map(|n| move_button(n.id, n.san.clone(), n.show_number, n.fullmove, n.is_white))
                    .collect_view();
                view! {
                    <div
                        class="flex flex-row items-center gap-1 py-0.5"
                        class:text-zinc-500=is_variation
                        class:text-sm=is_variation
                        style=indent.clone()
                    >
                        {(is_variation && i == 0).then(|| view! { <span class="select-none">"("</span> })}
                        {buttons}
                        {(is_variation && i == last).then(|| view! { <span class="select-none">")"</span> })}
                    </div>
                }
                .into_any()
            }
            Row::Nested(sub) => render_line(sub, true, move_button),
        })
        .collect_view();

    view! { <div class="flex flex-col gap-0.5">{children}</div> }.into_any()
}

/// Standalone forward/back/jump navigation, decoupled from the move list
/// itself — the compact (mobile) move strip used to bake these buttons
/// into the same horizontally-scrolling row as the moves, which made them
/// small and easy to fat-finger and tied their size to however much room
/// the list left over. A dedicated, full-width row of larger buttons is
/// friendlier to tap and doesn't compete with the list for space.
#[component]
pub fn MoveNavButtons(tree: RwSignal<MoveTree>, cursor: RwSignal<NodeId>) -> impl IntoView {
    let play_sound_for = move |id: NodeId| {
        tree.with_untracked(|t| {
            if id == t.root() {
                return;
            }
            let uci = t.uci(id);
            if let Ok(parsed) = uci.parse::<shakmaty::uci::UciMove>() {
                let parent_pos = t.position(t.parent(id).unwrap()).clone();
                if let Ok(m) = parsed.to_move(&parent_pos) {
                    sound::play(sound::for_move(t.position(id), &m));
                }
            }
        });
    };
    let go_to = move |id: NodeId| {
        cursor.set(id);
        play_sound_for(id);
    };
    let go_back = move |_| {
        let parent = tree.with_untracked(|t| t.parent(cursor.get_untracked()));
        if let Some(p) = parent {
            go_to(p);
        }
    };
    let go_forward = move |_| {
        let child = tree.with_untracked(|t| t.first_child(cursor.get_untracked()));
        if let Some(c) = child {
            go_to(c);
        }
    };
    let go_beginning = move |_| {
        let root = tree.with_untracked(MoveTree::root);
        go_to(root);
    };
    let go_end = move |_| {
        let end = tree.with_untracked(|t| t.last_mainline_from(t.root()));
        go_to(end);
    };
    let can_back = Signal::derive(move || cursor.get() != 0);
    let can_forward = Signal::derive(move || tree.with(|t| t.first_child(cursor.get()).is_some()));

    let btn_class = "flex-1 py-2.5 text-3xl leading-none font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:border-zinc-700 disabled:hover:text-zinc-300";

    view! {
        <div class="flex flex-row items-stretch gap-1.5 w-full">
            <button on:click=move |_| go_beginning(()) disabled=move || !can_back.get() aria-label="Jump to start" class=btn_class>"«"</button>
            <button on:click=move |_| go_back(()) disabled=move || !can_back.get() aria-label="Previous move" class=btn_class>"‹"</button>
            <button on:click=move |_| go_forward(()) disabled=move || !can_forward.get() aria-label="Next move" class=btn_class>"›"</button>
            <button on:click=move |_| go_end(()) disabled=move || !can_forward.get() aria-label="Jump to end" class=btn_class>"»"</button>
        </div>
    }
}

#[component]
pub fn AnalysisMovesPanel(
    tree: RwSignal<MoveTree>,
    cursor: RwSignal<NodeId>,
    /// Compact horizontal strip layout for mobile. Defaults to the full
    /// vertical panel layout suitable for a desktop side column.
    #[prop(optional)]
    compact: bool,
) -> impl IntoView {
    let segments = Signal::derive(move || tree.with(MoveTree::flatten));

    let path = Signal::derive(move || tree.with(|t| t.path_from_root(cursor.get())));
    let is_current = move |id: NodeId| path.with(|p| p.last() == Some(&id));

    let play_sound_for = move |id: NodeId| {
        tree.with_untracked(|t| {
            if id == t.root() {
                return;
            }
            // The move that reached `id` was played against its parent's
            // position; `sound::for_move` wants the resulting position.
            let uci = t.uci(id);
            if let Ok(parsed) = uci.parse::<shakmaty::uci::UciMove>() {
                let parent_pos = t.position(t.parent(id).unwrap()).clone();
                if let Ok(m) = parsed.to_move(&parent_pos) {
                    sound::play(sound::for_move(t.position(id), &m));
                }
            }
        });
    };

    let go_to = move |id: NodeId| {
        cursor.set(id);
        play_sound_for(id);
    };
    let go_back = move |_| {
        let parent = tree.with_untracked(|t| t.parent(cursor.get_untracked()));
        if let Some(p) = parent {
            go_to(p);
        }
    };
    let go_forward = move |_| {
        let child = tree.with_untracked(|t| t.first_child(cursor.get_untracked()));
        if let Some(c) = child {
            go_to(c);
        }
    };
    let go_beginning = move |_| {
        let root = tree.with_untracked(MoveTree::root);
        go_to(root);
    };
    let go_end = move |_| {
        let end = tree.with_untracked(|t| t.last_mainline_from(t.root()));
        go_to(end);
    };

    let can_back = Signal::derive(move || cursor.get() != 0);
    let can_forward = Signal::derive(move || {
        tree.with(|t| t.first_child(cursor.get()).is_some())
    });

    let promote_node = move |id: NodeId| {
        tree.update(|t| t.promote(id));
    };
    let delete_node = move |id: NodeId| {
        let parent_of_deleted = tree.with_untracked(|t| t.parent(id));
        let cursor_in_subtree = path.with_untracked(|p| p.contains(&id));
        tree.update(|t| t.delete(id));
        if cursor_in_subtree {
            if let Some(p) = parent_of_deleted {
                cursor.set(p);
            }
        }
    };

    // Both desktop (full) and mobile (compact) instances mount at once and
    // are toggled via CSS, so registering a window-level keydown listener in
    // each would fire navigation twice per press. Only the non-compact
    // instance owns the keyboard.
    #[cfg(feature = "hydrate")]
    if !compact {
        use leptos::ev;
        use leptos::prelude::window_event_listener;
        use wasm_bindgen::JsCast as _;

        window_event_listener(ev::keydown, move |e: web_sys::KeyboardEvent| {
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
                    go_end(());
                }
                "ArrowDown" => {
                    e.prevent_default();
                    go_beginning(());
                }
                _ => {}
            }
        });
    }

    // Only the non-compact (desktop) layout still embeds its own button
    // row below — compact's are a separate `MoveNavButtons` instance now.
    let nav_btn_class = "flex-1 px-2 py-1.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded hover:border-zinc-500 hover:text-white transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:border-zinc-700 disabled:hover:text-zinc-300";

    let move_button = move |id: NodeId, san: String, show_number: bool, fullmove: u32, is_white: bool| {
        let label = if show_number {
            format!("{fullmove}{} {san}", if is_white { "." } else { "..." })
        } else {
            san
        };
        view! {
            <button
                class="px-1.5 py-0.5 rounded hover:bg-zinc-700 cursor-pointer whitespace-nowrap"
                class:bg-zinc-700=move || is_current(id)
                on:click=move |_| go_to(id)
                on:contextmenu=move |e| {
                    e.prevent_default();
                    promote_node(id);
                }
                on:dblclick=move |_| delete_node(id)
                title="Click to jump here. Right-click to promote to mainline. Double-click to delete."
            >
                {label}
            </button>
        }.into_any()
    };

    // Compact (mobile) layout: a single horizontal scroll strip, one card
    // per segment side by side — already a reasonable model for variations
    // (you scroll past an aside rather than wrapping around it), so it keeps
    // the flat per-segment rendering.
    let body = move || {
        segments.with(|segs| {
            segs.iter()
                .map(|seg| {
                    let depth = seg.depth;
                    let is_variation = depth > 0;
                    let buttons = seg
                        .nodes
                        .iter()
                        .map(|n| move_button(n.id, n.san.clone(), n.show_number, n.fullmove, n.is_white))
                        .collect_view();
                    view! {
                        <div
                            class="flex flex-row flex-wrap items-center gap-x-1 gap-y-0.5 py-0.5"
                            class:text-zinc-500=is_variation
                            class:text-sm=is_variation
                            style=format!("padding-left: {}rem", depth as f64 * 1.0)
                        >
                            {is_variation.then(|| view! { <span class="select-none">"("</span> })}
                            {buttons}
                            {is_variation.then(|| view! { <span class="select-none">")"</span> })}
                        </div>
                    }
                })
                .collect_view()
        })
    };

    // Full (desktop) layout: the mainline — and each variation's own
    // continuation around any sub-variations — flows as one continuous,
    // naturally-wrapping set of moves. A variation only breaks that flow
    // where it actually branches off, rendering as its own indented block
    // (`basis-full` forces it onto a fresh line within the shared
    // `flex-wrap` container) directly below the move it came from, after
    // which the enclosing line picks back up.
    let full_body = move || tree.with(|t| render_line(&t.flatten_tree(), false, move_button));

    let is_empty = Signal::derive(move || segments.with(Vec::is_empty));

    if compact {
        // Just the scrolling move strip now — navigation is a separate
        // `MoveNavButtons` instance the caller places wherever makes sense
        // (see that component's doc comment for why).
        view! {
            <div class="w-full overflow-x-auto flex flex-row items-center gap-2 whitespace-nowrap text-xs font-mono rounded-md bg-zinc-900/60 border border-zinc-800 px-2 py-1.5">
                {body}
            </div>
        }.into_any()
    } else {
        view! {
            <div class="flex flex-col h-full min-h-0 text-sm font-mono">
                <div class="flex-1 min-h-0 overflow-y-auto flex flex-col gap-0.5">
                    <Show
                        when=move || !is_empty.get()
                        fallback=|| view! {
                            <div class="text-zinc-500 text-xs italic px-1 py-2">"No moves yet"</div>
                        }
                    >
                        {full_body}
                    </Show>
                </div>
                <div class="flex flex-row items-stretch gap-1 mt-2 pt-2 border-t border-zinc-800">
                    <button on:click=move |_| go_beginning(()) disabled=move || !can_back.get() aria-label="Jump to start" class=nav_btn_class>"«"</button>
                    <button on:click=move |_| go_back(()) disabled=move || !can_back.get() aria-label="Previous move" class=nav_btn_class>"‹"</button>
                    <button on:click=move |_| go_forward(()) disabled=move || !can_forward.get() aria-label="Next move" class=nav_btn_class>"›"</button>
                    <button on:click=move |_| go_end(()) disabled=move || !can_forward.get() aria-label="Jump to end" class=nav_btn_class>"»"</button>
                </div>
            </div>
        }.into_any()
    }
}
