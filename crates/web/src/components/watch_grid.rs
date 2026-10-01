use std::collections::HashMap;

use leptos::prelude::*;
use shared::WatchGameSummary;
use uuid::Uuid;

use crate::components::{BoardPerspective, ChessBoard, Clock};
use crate::watch::WATCH_GRID_LIMIT;

#[cfg(feature = "hydrate")]
fn parse_fen(fen: &str) -> Option<shakmaty::Chess> {
    fen.parse::<shakmaty::fen::Fen>()
        .ok()?
        .into_position::<shakmaty::Chess>(shakmaty::CastlingMode::Standard)
        .ok()
}

fn player_name(p: &shared::PlayerInfo) -> String {
    p.username.clone().unwrap_or_else(|| "Anonymous".to_string())
}

/// A tile's live-updating slice of a `WatchGameSummary` — position plus the
/// clock snapshot needed to keep ticking it down locally between updates
/// (see `Clock`). Refreshed wholesale by a `Position` push (a move just
/// happened, so position and clocks change together); a `Roster` tick only
/// seeds this the *first* time a game appears, deliberately never
/// overwriting it afterward — the same reasoning that already applied to
/// position alone before clocks existed here: a `Position` push is more
/// current than anything the next 3-second roster tick would say anyway.
#[derive(Clone, Default)]
struct TileState {
    position: shakmaty::Chess,
    white_ms_left: Option<i64>,
    black_ms_left: Option<i64>,
    sent_at_ms: i64,
}

#[component]
fn WatchTile(game: WatchGameSummary, tile: Signal<TileState>) -> impl IntoView {
    use shakmaty::{Color, Position as _};

    let last_move = RwSignal::new(None::<(shakmaty::Square, shakmaty::Square)>);
    let on_move = Callback::new(|_: shakmaty::Move| {});
    let on_premove = Callback::new(|_: (shakmaty::Square, shakmaty::Square)| {});
    let can_drag_piece = Callback::new(|_: shakmaty::Piece| false);
    let is_my_turn = Signal::derive(|| false);
    let href = format!("/game/{}", game.game_id);
    let category = game.category.to_string();
    let rated_label = if game.rated { "Rated" } else { "Casual" };

    let position = Signal::derive(move || tile.get().position);
    let sent_at_ms = Signal::derive(move || tile.get().sent_at_ms);
    let no_offset = Signal::derive(|| 0_i64);
    let no_abort = Signal::derive(|| None::<i64>);
    let turn = Signal::derive(move || tile.with(|t| t.position.turn()));
    let white_active = Signal::derive(move || turn.get() == Color::White);
    let black_active = Signal::derive(move || turn.get() == Color::Black);

    // Built once, not inside the view closure. These previously read as
    // `Signal::derive(move || ms)` over a plain `i64` captured from
    // `tile.get()`, which meant a brand-new constant signal — and its node in
    // the reactive graph — on every tile update, for every visible tile, every
    // few seconds.
    let white_ms = Signal::derive(move || tile.with(|t| t.white_ms_left.unwrap_or(0)));
    let black_ms = Signal::derive(move || tile.with(|t| t.black_ms_left.unwrap_or(0)));
    let has_white_clock = Signal::derive(move || tile.with(|t| t.white_ms_left.is_some()));
    let has_black_clock = Signal::derive(move || tile.with(|t| t.black_ms_left.is_some()));

    // Which side is to move, so the active player can be marked. A tile is
    // a tiny board at grid size — without this there is no way to tell whose
    // clock is running except by watching the digits tick.
    let white_to_move = Signal::derive(move || turn.get() == Color::White);

    view! {
        <a
            href={href}
            class="group flex flex-col surface-card overflow-hidden hover:border-zinc-700 transition-colors"
        >
            // ── Header: who is playing, and at what ───────────────────────
            // Moved above the board and given real hierarchy. Previously the
            // tile was almost entirely board, with one cramped row of names
            // underneath — you could see a position but not who was in it.
            <div class="flex items-center justify-between gap-2 px-3 pt-3 pb-2">
                <span class="eyebrow-sm text-zinc-500 truncate">
                    {category}
                </span>
                <span
                    class="eyebrow-sm flex-shrink-0"
                    class:text-zinc-500=!game.rated
                    class:accent-text=game.rated
                >
                    {rated_label}
                </span>
            </div>

            <ChessBoard
                position={position}
                perspective={Signal::derive(|| BoardPerspective::White)}
                last_move={last_move}
                on_move={on_move}
                on_premove={on_premove}
                can_drag_piece={can_drag_piece}
                is_my_turn={is_my_turn}
                size_class="w-full aspect-square"
            />

            // ── Players ───────────────────────────────────────────────────
            // One row per player, each with a colour chip, name, rating and
            // clock — and the side to move highlighted.
            <div class="flex flex-col divide-y divide-zinc-800/70">
                <PlayerRow
                    name=player_name(&game.white)
                    rating=game.white.rating
                    is_white=true
                    active=white_active
                    to_move=white_to_move
                    ms=white_ms
                    has_clock=has_white_clock
                    sent_at_ms=sent_at_ms
                    no_offset=no_offset
                    no_abort=no_abort
                />
                <PlayerRow
                    name=player_name(&game.black)
                    rating=game.black.rating
                    is_white=false
                    active=black_active
                    to_move=Signal::derive(move || !white_to_move.get())
                    ms=black_ms
                    has_clock=has_black_clock
                    sent_at_ms=sent_at_ms
                    no_offset=no_offset
                    no_abort=no_abort
                />
            </div>
        </a>
    }
}

/// One player's line on a watch tile.
#[component]
#[allow(clippy::too_many_arguments)]
fn PlayerRow(
    name: String,
    rating: i32,
    is_white: bool,
    active: Signal<bool>,
    /// Whether it is this player's turn — drives the emphasis, so a glance at
    /// the tile says who is thinking.
    to_move: Signal<bool>,
    ms: Signal<i64>,
    has_clock: Signal<bool>,
    sent_at_ms: Signal<i64>,
    no_offset: Signal<i64>,
    no_abort: Signal<Option<i64>>,
) -> impl IntoView {
    view! {
        <div
            class="flex items-center justify-between gap-2 px-3 py-2 transition-colors"
            class:bg-zinc-800=move || to_move.get()
        >
            <div class="flex items-center gap-2 min-w-0">
                // Colour chip, using the live board tokens so it matches
                // whatever board theme the viewer has chosen.
                <span
                    class="w-2.5 h-2.5 rounded-sm flex-shrink-0 border border-zinc-700"
                    class:sq-light=is_white
                    class:sq-dark=!is_white
                />
                <span
                    class="text-xs truncate"
                    class:text-white=move || to_move.get()
                    class:font-semibold=move || to_move.get()
                    class:text-zinc-400=move || !to_move.get()
                >
                    {name}
                </span>
                <span class="text-[10px] text-zinc-500 flex-shrink-0">{rating}</span>
            </div>
            <div class="flex-shrink-0 text-xs">
                <Show
                    when=move || has_clock.get()
                    fallback=|| view! { <span class="text-zinc-600">"\u{2014}"</span> }
                >
                    <Clock
                        snapshot_ms={ms}
                        snapshot_sent_at_ms={sent_at_ms}
                        is_active={active}
                        offset_ms={no_offset}
                        abort_deadline_ms={no_abort}
                    />
                </Show>
            </div>
        </div>
    }
}

#[component]
pub fn WatchGrid() -> impl IntoView {
    let games = RwSignal::new(Vec::<WatchGameSummary>::new());
    let total_active = RwSignal::new(0usize);
    let tiles = RwSignal::new(HashMap::<Uuid, TileState>::new());
    // Distinct from `games` being empty — that's also true before the first
    // `Roster` message ever arrives, which would otherwise flash "No games
    // in progress" on every load regardless of whether that's actually true
    // yet. Set once, on the first Roster (even an empty one still counts as
    // "loaded"); never reset — a reconnect refreshes `games` in place
    // without a visible loading flicker each time.
    let has_loaded = RwSignal::new(false);

    #[cfg(feature = "hydrate")]
    {
        use futures::channel::mpsc;
        use futures::StreamExt;
        use gloo_timers::future::TimeoutFuture;
        use leptos::task::spawn_local;
        use shared::{WatchClientMessage, WatchServerMessage};

        use crate::watch::watch_websocket;

        let cancelled = StoredValue::new(false);

        spawn_local(async move {
            let mut backoff_ms = 500u32;
            loop {
                if cancelled.get_value() {
                    break;
                }

                let (mut tx, rx) = mpsc::channel::<WatchClientMessage>(1);
                let _ = tx.try_send(WatchClientMessage::Connect);

                match watch_websocket(rx.map(Ok).into()).await {
                    Ok(mut messages) => {
                        backoff_ms = 500;
                        while let Some(msg) = messages.next().await {
                            let Ok(msg) = msg else { continue };
                            match msg {
                                WatchServerMessage::Roster {
                                    games: new_games,
                                    total_active: t,
                                } => {
                                    tiles.update(|map| {
                                        let ids: std::collections::HashSet<Uuid> =
                                            new_games.iter().map(|g| g.game_id).collect();
                                        map.retain(|id, _| ids.contains(id));
                                        // Only seeds a game's *first* appearance —
                                        // a `Position` push is always more current
                                        // than the next periodic tick, so an
                                        // already-present entry is deliberately
                                        // left untouched here (see `TileState`'s
                                        // doc comment).
                                        for g in &new_games {
                                            map.entry(g.game_id).or_insert_with(|| TileState {
                                                position: parse_fen(&g.fen).unwrap_or_default(),
                                                white_ms_left: g.white_ms_left,
                                                black_ms_left: g.black_ms_left,
                                                sent_at_ms: g.sent_at_ms,
                                            });
                                        }
                                    });
                                    games.set(new_games);
                                    total_active.set(t);
                                    has_loaded.set(true);
                                }
                                WatchServerMessage::Position {
                                    game_id,
                                    fen,
                                    white_ms_left,
                                    black_ms_left,
                                    sent_at_ms,
                                } => {
                                    if let Some(chess) = parse_fen(&fen) {
                                        tiles.update(|map| {
                                            map.insert(game_id, TileState {
                                                position: chess,
                                                white_ms_left: Some(white_ms_left),
                                                black_ms_left: Some(black_ms_left),
                                                sent_at_ms,
                                            });
                                        });
                                    }
                                }
                            }
                        }
                        if cancelled.get_value() {
                            break;
                        }
                    }
                    Err(e) => leptos::logging::warn!("watch websocket error: {e}"),
                }

                TimeoutFuture::new(backoff_ms).await;
                backoff_ms = (backoff_ms * 2).min(5000);
            }
        });

        on_cleanup(move || {
            cancelled.set_value(true);
        });
    }

    view! {
        <div class="max-w-6xl mx-auto px-6 py-8">
            <div class="flex items-end justify-between gap-4 mb-6">
                <div class="flex flex-col gap-1">
                    <h1 class="display-1 text-white">"Watch"</h1>
                    // Always shown once loaded, not only when the roster is
                    // truncated — "12 games in progress" is useful context on
                    // its own, and the old conditional meant the header
                    // silently changed shape as the count crossed the cap.
                    <Show when=move || has_loaded.get()>
                        <span class="text-xs text-zinc-500">
                            {move || {
                                let shown = games.get().len();
                                let total = total_active.get();
                                if total > WATCH_GRID_LIMIT {
                                    format!("Showing the top {shown} of {total} games in progress")
                                } else if total == 1 {
                                    "1 game in progress".to_string()
                                } else {
                                    format!("{total} games in progress")
                                }
                            }}
                        </span>
                    </Show>
                </div>
            </div>
            <Show
                when=move || has_loaded.get()
                fallback=|| view! {
                    <div class="flex flex-col items-center justify-center gap-3 py-16 text-zinc-500">
                        <div class="w-8 h-8 rounded-full border-2 border-zinc-700 border-t-zinc-400 animate-spin"></div>
                        <p class="text-sm">"Loading active games..."</p>
                    </div>
                }
            >
                <Show
                    when=move || !games.get().is_empty()
                    fallback=|| view! {
                        <p class="text-zinc-500 text-sm italic px-1">"No games in progress right now"</p>
                    }
                >
                    <div class="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-5">
                        <For
                            each=move || games.get()
                            key=|g| g.game_id
                            let(game)
                        >
                            {
                                let id = game.game_id;
                                let tile = Signal::derive(move || {
                                    tiles.get().get(&id).cloned().unwrap_or_default()
                                });
                                view! { <WatchTile game={game} tile={tile} /> }
                            }
                        </For>
                    </div>
                </Show>
            </Show>
        </div>
    }
}
