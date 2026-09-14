mod engine;
mod moves_tree;
mod openings;
mod tree;

pub use engine::{EngineHandle, EngineInfo, Score};
pub use moves_tree::AnalysisMovesPanel;
pub use tree::{DisplayNode, LineItem, MoveLine, MoveTree, NodeId, Segment};

use leptos::prelude::*;
use shakmaty::{Chess, Color, EnPassantMode};
use uuid::Uuid;

use engine::{format_white_score, pv_first_move, pv_to_san, to_white_pov, white_fraction};

use crate::components::chess_board::ChessBoard;
use crate::components::{material_advantage, BoardPerspective, CapturedPieces, Clock};
use crate::game::get_game_for_analysis;
use crate::sound;

/// One MultiPV line, fully processed for display at the moment it's
/// received — converted to White's POV and replayed to SAN against the
/// exact position the engine actually searched (both tagged onto the
/// `EngineInfo` by `EngineHandle` itself, via its `current_fen`). Doing
/// this once at ingestion, rather than re-deriving it at render time
/// against whatever position is on screen *now*, is what keeps the eval
/// bar/candidates/PV from flashing a wrong (often sign-inverted) value for
/// an instant on every move — the board's cursor moves the instant a move
/// is played, but engine results for the position just left keep arriving
/// briefly after, and re-converting those against the new position is
/// exactly the mismatch that caused it.
#[derive(Debug, Clone)]
struct DisplayLine {
    score_white: Score,
    depth: u32,
    san: Vec<String>,
    /// The first move's (from, to) squares, for drawing a best-move arrow —
    /// `None` if the PV was empty or malformed (see `engine::pv_first_move`).
    arrow: Option<(shakmaty::Square, shakmaty::Square)>,
}

/// A puzzle's move line to preload into the analysis board — see
/// `pages::puzzles`'s "Open in analysis" button. Distinct from `game_id`:
/// puzzles have no DB row, so this carries everything needed directly
/// rather than triggering a server fetch.
#[derive(Clone, PartialEq)]
pub struct PuzzleLoad {
    pub fen: String,
    pub moves: Vec<String>,
    /// How many plies from `fen` to open the cursor at.
    pub ply: usize,
}

/// Analysis board: a client-only board where the local user moves both
/// colors, rewinds through history, and branches into variations. No
/// network, no auth — see `PlayBoard` for the live-game equivalent this
/// borrows its layout from.
///
/// When `game_id` resolves to `Some`, a finished (or aborted) game's stored
/// moves and clocks are loaded in, replacing whatever's on the board. When
/// `puzzle` resolves to `Some`, its move line is loaded the same way, just
/// from client-supplied data instead of a DB fetch.
#[component]
pub fn AnalysisBoard(
    #[prop(optional, into)] game_id: Signal<Option<Uuid>>,
    #[prop(optional, into)] puzzle: Signal<Option<PuzzleLoad>>,
) -> impl IntoView {
    let tree = RwSignal::new(MoveTree::new(Chess::default()));
    let cursor = RwSignal::new(0_usize);
    let flipped = RwSignal::new(false);
    let last_move = RwSignal::new(None::<(shakmaty::Square, shakmaty::Square)>);

    // Load a stored game once `game_id` resolves. Mirrors `AnalysisControls`'
    // PGN-import path: replace the tree, jump the cursor to the end of the
    // mainline.
    let game_data = Resource::new(
        move || game_id.get(),
        |id| async move {
            match id {
                Some(id) => get_game_for_analysis(id).await.ok(),
                None => None,
            }
        },
    );
    Effect::new(move || {
        if let Some(data) = game_data.get().flatten() {
            if let Ok(loaded) = MoveTree::from_uci_moves(&data.moves, &data.clocks, data.initial_time_ms) {
                let end = loaded.last_mainline_from(loaded.root());
                tree.set(loaded);
                cursor.set(end);
            }
        }
    });

    // Load a puzzle's move line once `puzzle` resolves. Independent of the
    // `game_id` path above — a page only ever supplies one or the other.
    Effect::new(move || {
        let Some(load) = puzzle.get() else { return };
        let Some(root) = load
            .fen
            .parse::<shakmaty::fen::Fen>()
            .ok()
            .and_then(|f| f.into_position::<Chess>(shakmaty::CastlingMode::Standard).ok())
        else {
            return;
        };
        let Ok(loaded) = MoveTree::from_uci_moves_at(root, &load.moves) else {
            return;
        };

        let mut node = loaded.root();
        let mut last = None;
        for _ in 0..load.ply {
            let Some(next) = loaded.first_child(node) else { break };
            // from/to for the highlight, resolved the same way every other
            // move-application site in this codebase does it.
            let mv = loaded
                .uci(next)
                .parse::<shakmaty::uci::UciMove>()
                .ok()
                .and_then(|u| u.to_move(loaded.position(node)).ok());
            if let Some(mv) = mv {
                last = mv.from().map(|f| (f, mv.to()));
            }
            node = next;
        }

        tree.set(loaded);
        cursor.set(node);
        last_move.set(last);
    });

    let position = Signal::derive(move || tree.with(|t| t.position(cursor.get()).clone()));
    let perspective = Signal::derive(move || {
        if flipped.get() {
            BoardPerspective::Black
        } else {
            BoardPerspective::White
        }
    });

    let on_move = Callback::new(move |m: shakmaty::Move| {
        let at = cursor.get_untracked();
        let Some(new_id) = tree.try_update(|t| t.play(at, m, None)) else {
            return;
        };
        if let Some(from) = m.from() {
            last_move.set(Some((from, m.to())));
        }
        let new_pos = tree.with_untracked(|t| t.position(new_id).clone());
        sound::play(sound::for_move(&new_pos, &m));
        cursor.set(new_id);
    });
    // is_my_turn is always true (both colors are the local user's to move),
    // so ChessBoard/Square never take the premove branch — this is
    // unreachable, but ChessBoard requires the callback regardless.
    let on_premove = Callback::new(|_: (shakmaty::Square, shakmaty::Square)| {});
    let can_drag_piece = Callback::new(|_: shakmaty::Piece| true);
    let is_my_turn = Signal::derive(|| true);

    let top_color = Signal::derive(move || match perspective.get() {
        BoardPerspective::White => Color::Black,
        BoardPerspective::Black => Color::White,
    });
    let bottom_color = Signal::derive(move || match perspective.get() {
        BoardPerspective::White => Color::White,
        BoardPerspective::Black => Color::Black,
    });
    let top_advantage = Signal::derive(move || material_advantage(&position.get(), top_color.get()));
    let bottom_advantage = Signal::derive(move || -top_advantage.get());

    // Clock at the position being viewed, inherited from the nearest
    // ancestor that recorded one (e.g. a loaded PGN's `%clk` comments, or a
    // gambit.rs game's own history once that import path exists). `None`
    // when nothing in scope ever carried clock data — most analysis starts
    // from a blank board, so the clock widgets simply don't render then.
    let clocks = Signal::derive(move || tree.with(|t| t.clock_at(cursor.get())));
    let has_clocks = Signal::derive(move || clocks.get().is_some());
    let top_ms = Signal::derive(move || {
        let (w, b) = clocks.get().unwrap_or((0, 0));
        match perspective.get() {
            BoardPerspective::White => b,
            BoardPerspective::Black => w,
        }
    });
    let bottom_ms = Signal::derive(move || {
        let (w, b) = clocks.get().unwrap_or((0, 0));
        match perspective.get() {
            BoardPerspective::White => w,
            BoardPerspective::Black => b,
        }
    });
    let clock_inactive = Signal::derive(|| false);
    let no_offset = Signal::derive(|| 0_i64);
    let no_abort = Signal::derive(|| None::<i64>);

    // Stockfish runs client-side in a Web Worker (see engine.rs), started
    // as soon as the board mounts (`engine_on` defaults to `true`) — the
    // ~7MB download happens automatically rather than waiting for a
    // manual toggle; the "Engine: On/Off" button still lets it be turned
    // off. `engine` itself is an inert stub outside the browser
    // (`EngineHandle` on non-hydrate targets), so none of this needs
    // `#[cfg(feature = "hydrate")]` splitting.
    let engine: StoredValue<Option<EngineHandle>, LocalStorage> = StoredValue::new_local(None);
    let engine_on = RwSignal::new(true);
    // Slot `i` holds the engine's `multipv` line `i + 1` — 3 concurrent
    // candidate lines (see `EngineHandle::new`'s `setoption MultiPV`), best
    // line first. Slots fill in independently as each line's `info` arrives,
    // so a slot briefly holding the previous position's line while a fresh
    // search is in flight is expected (same staleness the single-line eval
    // always had), not a bug.
    let lines = RwSignal::new([None::<DisplayLine>, None, None]);
    // Bumped whenever the worker reports its own crash (`Worker::onerror`)
    // — read inside the effect below purely to make it re-run, since the
    // vendored Stockfish WASM can hit an internal trap on its own during
    // ordinary use (not something reachable from our side to prevent; see
    // `EngineHandle::new`'s `on_crash` doc comment) and the only recovery
    // is to drop the dead worker and start a fresh one.
    let engine_generation = RwSignal::new(0u32);

    Effect::new(move || {
        if !engine_on.get() {
            return;
        }
        let _ = engine_generation.get();
        let fen = shakmaty::fen::Fen::from_position(&position.get(), EnPassantMode::Legal).to_string();

        engine.update_value(|e| {
            if e.is_none() {
                lines.set([None, None, None]);
                let on_crash = move || {
                    engine.update_value(|e| *e = None);
                    engine_generation.update(|g| *g += 1);
                };
                let on_info = move |info: EngineInfo, fen: String| {
                    use shakmaty::Position as _;
                    // `fen` is exactly the position `EngineHandle` searched
                    // to produce `info` (see `SearchState::current_fen`) —
                    // not necessarily the position on screen right now, so
                    // parse and convert/replay against *that*, not
                    // `position.get()`.
                    let Some(searched) = fen.parse::<shakmaty::fen::Fen>().ok().and_then(|f| {
                        f.into_position::<Chess>(shakmaty::CastlingMode::Standard).ok()
                    }) else {
                        return;
                    };
                    let idx = (info.multipv.saturating_sub(1)).min(2) as usize;
                    let white_to_move = searched.turn() == Color::White;
                    let line = DisplayLine {
                        score_white: to_white_pov(info.score, white_to_move),
                        depth: info.depth,
                        san: pv_to_san(&searched, &info.pv),
                        arrow: pv_first_move(&searched, &info.pv),
                    };
                    lines.update(|l| l[idx] = Some(line));
                };
                match EngineHandle::new(on_info, on_crash) {
                    Ok(h) => *e = Some(h),
                    Err(err) => {
                        leptos::logging::warn!("failed to start engine: {err}");
                        engine_on.set(false);
                        return;
                    }
                }
            }
            // `go` itself serializes searches through the engine's own
            // `bestmove` acknowledgement (see `engine.rs`), so calling it
            // on every position change needs no debouncing here — at most
            // one search is ever in flight, and a burst of rapid changes
            // just keeps replacing which position is queued up next.
            if let Some(h) = e {
                h.go(&fen);
            }
        });
    });

    on_cleanup(move || {
        engine.update_value(|e| *e = None);
    });

    // The best (multipv 1) line's eval, already normalized to White's POV
    // at ingestion (see the `on_info` closure above) — shared by `EvalBar`
    // and `CandidateMoves` so every display reads the exact same number
    // rather than each re-deriving it.
    let white_score = Signal::derive(move || lines.get()[0].clone().map(|l| l.score_white));

    // One arrow per candidate line that has a move to show, ranked by
    // MultiPV slot (0 = best) so `ChessBoard` can fade the others out.
    let arrows = Signal::derive(move || {
        lines
            .get()
            .into_iter()
            .enumerate()
            .filter_map(|(i, l)| l.and_then(|l| l.arrow).map(|(from, to)| (from, to, i)))
            .collect::<Vec<_>>()
    });

    // Static opening-name lookup, independent of the engine — needs no
    // Stockfish at all, so it's shown regardless of `engine_on`.
    let opening = Signal::derive(move || tree.with(|t| openings::lookup(t, cursor.get())));

    view! {
        <div class="flex flex-col items-center justify-center w-full py-2 h-[calc(100dvh-3.5rem)]">
            <div class="flex flex-row items-stretch gap-2">
                <EvalBar white_score=white_score engine_on=engine_on perspective=perspective />
                <div class="relative w-[min(calc(100vw-2.25rem),calc(100dvh-15rem))] md:w-[min(calc(100vw-2.75rem),calc(100dvh-12.5rem))]">
                // Top row
                <div class="flex flex-row items-center pl-2 py-2 gap-2 overflow-hidden">
                    {move || view! { <CapturedPieces position={position} color={top_color.get()} /> }}
                    {move || (top_advantage.get() > 0).then(|| view! {
                        <span class="text-xs font-semibold text-zinc-400 flex-shrink-0">
                            {format!("+{}", top_advantage.get())}
                        </span>
                    })}
                    <div class="ml-auto">
                        <Show when=move || has_clocks.get()>
                            <Clock
                                snapshot_ms={top_ms}
                                snapshot_sent_at_ms={no_offset}
                                is_active={clock_inactive}
                                offset_ms={no_offset}
                                abort_deadline_ms={no_abort}
                            />
                        </Show>
                    </div>
                </div>
                <ChessBoard
                    position={position}
                    perspective={perspective}
                    last_move={last_move}
                    on_move={on_move}
                    on_premove={on_premove}
                    can_drag_piece={can_drag_piece}
                    is_my_turn={is_my_turn}
                    arrows={arrows}
                />
                // Bottom row
                <div class="flex flex-row items-center pl-2 py-2 gap-2 overflow-hidden">
                    {move || view! { <CapturedPieces position={position} color={bottom_color.get()} /> }}
                    {move || (bottom_advantage.get() > 0).then(|| view! {
                        <span class="text-xs font-semibold text-zinc-400 flex-shrink-0">
                            {format!("+{}", bottom_advantage.get())}
                        </span>
                    })}
                    <div class="ml-auto">
                        <Show when=move || has_clocks.get()>
                            <Clock
                                snapshot_ms={bottom_ms}
                                snapshot_sent_at_ms={no_offset}
                                is_active={clock_inactive}
                                offset_ms={no_offset}
                                abort_deadline_ms={no_abort}
                            />
                        </Show>
                    </div>
                </div>
                // Mobile-only compact moves strip + controls
                <div class="md:hidden px-2 pb-1 flex flex-col gap-2">
                    <OpeningName opening=opening />
                    <CandidateMoves lines=lines engine_on=engine_on />
                    <AnalysisMovesPanel tree=tree cursor=cursor compact=true />
                    <AnalysisControls tree=tree cursor=cursor flipped=flipped last_move=last_move position=position engine_on=engine_on />
                </div>
                // Desktop side column: move list + controls. Same
                // board-height pinning as `EvalBar` (see its comment) —
                // `top-1/2 -translate-y-1/2` centers this fixed-height
                // column within the parent's taller (top/bottom-row-
                // inclusive) height, landing flush with the board's actual
                // top/bottom edges.
                <div class="hidden md:flex absolute top-1/2 -translate-y-1/2 left-full ml-4 w-64 h-[min(100vw,calc(100dvh-15rem))] md:h-[min(100vw,calc(100dvh-12.5rem))] flex-col gap-3">
                    <OpeningName opening=opening />
                    <CandidateMoves lines=lines engine_on=engine_on />
                    <div class="flex-1 min-h-0 flex flex-col rounded-md bg-zinc-900/60 border border-zinc-800 p-2">
                        <AnalysisMovesPanel tree=tree cursor=cursor />
                    </div>
                    <AnalysisControls tree=tree cursor=cursor flipped=flipped last_move=last_move position=position engine_on=engine_on />
                </div>
                </div>
            </div>
        </div>
    }
}

/// The opening name (and ECO code) for the position being viewed, from the
/// real lichess opening database in `openings.rs` — needs no engine, so it
/// shows whether or not Stockfish is toggled on. Empty once play has left
/// the database's coverage.
#[component]
fn OpeningName(opening: Signal<Option<(String, String)>>) -> impl IntoView {
    view! {
        <Show when=move || opening.get().is_some()>
            <div class="rounded-md bg-zinc-900/60 border border-zinc-800 p-2 text-xs font-mono text-zinc-300">
                {move || opening.get().map(|(eco, name)| format!("{eco} · {name}"))}
            </div>
        </Show>
    }
}

/// The top 3 candidate moves (MultiPV), each with its resulting eval,
/// search depth, and full continuation — everything `EngineEval` used to
/// show for the single best line alone, now per-line here instead (that
/// separate box is gone; this replaces it).
#[component]
fn CandidateMoves(lines: RwSignal<[Option<DisplayLine>; 3]>, engine_on: RwSignal<bool>) -> impl IntoView {
    view! {
        <Show when=move || engine_on.get()>
            <div class="rounded-md bg-zinc-900/60 border border-zinc-800 p-2 text-xs font-mono flex flex-col gap-2">
                {move || {
                    let lines = lines.get();
                    if lines.iter().all(Option::is_none) {
                        return view! { <span class="text-zinc-500 italic">"thinking…"</span> }.into_any();
                    }
                    lines
                        .into_iter()
                        .enumerate()
                        .filter_map(|(i, line)| line.map(|line| (i, line)))
                        .map(|(i, line)| {
                            let mv = line.san.first().cloned().unwrap_or_default();
                            let continuation = format!("depth {} — {}", line.depth, line.san.join(" "));
                            view! {
                                <div class="flex flex-col gap-0.5 min-w-0">
                                    <div class="flex flex-row items-center gap-2">
                                        <span class="text-zinc-500">{format!("{}.", i + 1)}</span>
                                        <span class="text-zinc-400 w-10">{format_white_score(line.score_white)}</span>
                                        <span class="text-white font-semibold">{mv}</span>
                                    </div>
                                    <div class="text-zinc-500 truncate pl-6">{continuation}</div>
                                </div>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </div>
        </Show>
    }
}

/// Vertical eval bar (chess.com-style) sitting to the left of the board:
/// White's share of the position fills from whichever edge White currently
/// sits at — the bottom when the board shows White's perspective, the top
/// when flipped — so the bar always lines up with the physical side of the
/// board each color occupies. The fill animates via a CSS `height`
/// transition rather than snapping on every update, and the eval number
/// itself sits inside whichever color's segment currently has the
/// advantage, matching chess.com's own bar.
#[component]
fn EvalBar(
    white_score: Signal<Option<Score>>,
    engine_on: RwSignal<bool>,
    perspective: Signal<BoardPerspective>,
) -> impl IntoView {
    let white_pct = Signal::derive(move || {
        white_score.get().map(white_fraction).unwrap_or(0.5) * 100.0
    });
    let label = Signal::derive(move || white_score.get().map(format_white_score));
    let white_at_bottom = Signal::derive(move || perspective.get() == BoardPerspective::White);
    let white_advantage = Signal::derive(move || white_pct.get() >= 50.0);
    // The label sits on whichever color is ahead, positioned at that
    // color's actual edge of the (possibly flipped) bar.
    let label_at_bottom = Signal::derive(move || white_advantage.get() == white_at_bottom.get());

    // `ChessBoard` requests `w-[min(100vw,calc(100dvh-11.5rem))]`, but it's
    // shrunk by its actual flex parent below — the `relative w-[...]` div
    // in `AnalysisBoard`'s own view — so *that* formula, not `ChessBoard`'s
    // own, is what actually governs the board's rendered size. Matching it
    // here (rather than `ChessBoard`'s formula) pins the bar to the board's
    // true height regardless of the top/bottom capture-piece rows' height
    // — rather than stretching to match this flex row's full height (which
    // includes those rows) the way it did before. `self-center` then
    // centers it within that taller row, landing flush against the
    // board's top/bottom edges since those rows are symmetric.
    //
    // The `vw` term also has this bar's own width (`w-7`/`w-9`) plus the
    // `gap-2` between it and the board subtracted out — the sibling board
    // div does the same subtraction for the same reason: without it, on a
    // tall/narrow viewport where the height cap doesn't bind, the board
    // alone would claim the full `100vw` and this bar would overflow the
    // row off the left edge instead of actually sitting beside the board.
    view! {
        <Show when=move || engine_on.get()>
            <div class="relative self-center w-7 md:w-9 h-[min(calc(100vw-2.25rem),calc(100dvh-15rem))] md:h-[min(calc(100vw-2.75rem),calc(100dvh-12.5rem))] flex-shrink-0 rounded-md overflow-hidden bg-zinc-900 border border-zinc-800">
                <div class="absolute inset-0 bg-zinc-950"></div>
                <div
                    class="absolute inset-x-0 bg-white transition-[height] duration-500 ease-out"
                    style=move || {
                        let pct = white_pct.get();
                        if white_at_bottom.get() {
                            format!("top: auto; bottom: 0; height: {pct}%")
                        } else {
                            format!("top: 0; bottom: auto; height: {pct}%")
                        }
                    }
                ></div>
                <span
                    class="absolute inset-x-0 text-center text-[10px] leading-none font-bold select-none"
                    class:bottom-1=label_at_bottom
                    class:top-1=move || !label_at_bottom.get()
                    class:text-zinc-950=white_advantage
                    class:text-white=move || !white_advantage.get()
                >
                    {move || label.get().unwrap_or_default()}
                </span>
            </div>
        </Show>
    }
}

#[component]
fn AnalysisControls(
    tree: RwSignal<MoveTree>,
    cursor: RwSignal<NodeId>,
    flipped: RwSignal<bool>,
    last_move: RwSignal<Option<(shakmaty::Square, shakmaty::Square)>>,
    #[prop(into)] position: Signal<shakmaty::Chess>,
    engine_on: RwSignal<bool>,
) -> impl IntoView {
    let load_open = RwSignal::new(false);
    let load_text = RwSignal::new(String::new());
    let load_error = RwSignal::new(None::<String>);

    let flip = move |_| flipped.update(|f| *f = !*f);
    let reset = move |_| {
        tree.set(MoveTree::new(Chess::default()));
        cursor.set(0);
        last_move.set(None);
        load_error.set(None);
    };

    let copy_fen = move |_: leptos::ev::MouseEvent| {
        let fen_str = shakmaty::fen::Fen::from_position(&position.get_untracked(), EnPassantMode::Legal)
            .to_string();
        #[cfg(feature = "hydrate")]
        if let Some(win) = web_sys::window() {
            let _ = win.navigator().clipboard().write_text(&fen_str);
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = fen_str;
    };
    let copy_pgn = move |_: leptos::ev::MouseEvent| {
        let pgn = tree.with_untracked(MoveTree::to_pgn);
        #[cfg(feature = "hydrate")]
        if let Some(win) = web_sys::window() {
            let _ = win.navigator().clipboard().write_text(&pgn);
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = pgn;
    };

    let do_load = move |_| {
        let text = load_text.get_untracked();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            load_error.set(Some("paste a FEN or PGN first".to_string()));
            return;
        }

        let fen_position = trimmed
            .parse::<shakmaty::fen::Fen>()
            .ok()
            .and_then(|fen| {
                fen.into_position::<shakmaty::Chess>(shakmaty::CastlingMode::Standard)
                    .ok()
            });
        if let Some(pos) = fen_position {
            tree.set(MoveTree::new(pos));
            cursor.set(0);
            last_move.set(None);
            load_error.set(None);
            load_open.set(false);
            load_text.set(String::new());
            return;
        }

        match MoveTree::from_pgn(trimmed) {
            Ok(new_tree) => {
                let end = new_tree.last_mainline_from(new_tree.root());
                tree.set(new_tree);
                cursor.set(end);
                last_move.set(None);
                load_error.set(None);
                load_open.set(false);
                load_text.set(String::new());
            }
            Err(e) => load_error.set(Some(format!("not a valid FEN or PGN: {e}"))),
        }
    };

    let btn_class = "px-2 py-1.5 text-xs font-medium text-zinc-300 border border-zinc-700 rounded hover:border-zinc-500 hover:text-white transition-colors cursor-pointer";

    view! {
        <div class="flex flex-col gap-2">
            <div class="flex flex-row flex-wrap items-stretch gap-1">
                <button
                    class=btn_class
                    class:bg-zinc-700=move || engine_on.get()
                    on:click=move |_| engine_on.update(|on| *on = !*on)
                >
                    {move || if engine_on.get() { "Engine: On" } else { "Engine: Off" }}
                </button>
                <button class=btn_class on:click=flip>"Flip"</button>
                <button class=btn_class on:click=reset>"Reset"</button>
                <button class=btn_class on:click=copy_fen>"Copy FEN"</button>
                <button class=btn_class on:click=copy_pgn>"Copy PGN"</button>
                <button class=btn_class on:click=move |_| load_open.update(|o| *o = !*o)>
                    {move || if load_open.get() { "Cancel" } else { "Load" }}
                </button>
            </div>
            <Show when=move || load_open.get()>
                <div class="flex flex-col gap-1">
                    <textarea
                        class="w-full h-24 text-xs font-mono bg-zinc-900 border border-zinc-700 rounded p-2 text-zinc-200 resize-none"
                        placeholder="Paste a FEN or PGN…"
                        prop:value=move || load_text.get()
                        on:input=move |e| load_text.set(event_target_value(&e))
                    ></textarea>
                    {move || load_error.get().map(|e| view! {
                        <span class="text-xs text-red-400">{e}</span>
                    })}
                    <button
                        class="px-2 py-1.5 text-xs font-medium bg-zinc-700 text-white rounded hover:bg-zinc-600 transition-colors cursor-pointer"
                        on:click=do_load
                    >
                        "Load position"
                    </button>
                </div>
            </Show>
        </div>
    }
}
