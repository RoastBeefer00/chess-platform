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
    let white_active =
        Signal::derive(move || tile.get().position.turn() == Color::White);
    let black_active =
        Signal::derive(move || tile.get().position.turn() == Color::Black);

    view! {
        <a
            href={href}
            class="flex flex-col gap-3 rounded-xl bg-zinc-900 border border-zinc-800/60 p-4 hover:border-zinc-700 transition-colors"
        >
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
            <div class="flex items-center justify-between text-xs gap-2">
                <div class="flex flex-col gap-1.5 min-w-0">
                    <span class="text-zinc-300 truncate">
                        {player_name(&game.white)} " " {game.white.rating}
                    </span>
                    <span class="text-zinc-500 truncate">
                        {player_name(&game.black)} " " {game.black.rating}
                    </span>
                </div>
                <div class="flex flex-col gap-1.5 flex-shrink-0">
                    {move || tile.get().white_ms_left.map(|ms| {
                        let ms_signal = Signal::derive(move || ms);
                        view! {
                            <Clock
                                snapshot_ms={ms_signal}
                                snapshot_sent_at_ms={sent_at_ms}
                                is_active={white_active}
                                offset_ms={no_offset}
                                abort_deadline_ms={no_abort}
                            />
                        }
                    })}
                    {move || tile.get().black_ms_left.map(|ms| {
                        let ms_signal = Signal::derive(move || ms);
                        view! {
                            <Clock
                                snapshot_ms={ms_signal}
                                snapshot_sent_at_ms={sent_at_ms}
                                is_active={black_active}
                                offset_ms={no_offset}
                                abort_deadline_ms={no_abort}
                            />
                        }
                    })}
                </div>
                <div class="flex flex-col items-end gap-0.5 flex-shrink-0">
                    <span class="text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-500">
                        {category}
                    </span>
                    <span class="text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-500">
                        {rated_label}
                    </span>
                </div>
            </div>
        </a>
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

    // Bound as a named closure rather than inlined into the `when=` attribute
    // below: an unparenthesized `>` there is ambiguous with the RSX tag-close
    // `>` for the `view!` macro's parser, and removing the disambiguating
    // parens (as `unused_parens` otherwise suggests) breaks the parse.
    let has_more_than_shown = move || total_active.get() > WATCH_GRID_LIMIT;

    view! {
        <div class="max-w-5xl mx-auto px-6 py-8">
            <div class="flex items-center justify-between mb-6">
                <h1 class="text-3xl font-bold tracking-tighter text-white">"Watch"</h1>
                <Show when=has_more_than_shown>
                    <span class="text-xs text-zinc-500">
                        {move || format!("Showing {} of {} active games", games.get().len(), total_active.get())}
                    </span>
                </Show>
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
                    <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-5">
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
