use leptos::prelude::*;
use leptos_router::{lazy_route, LazyRoute};
use shakmaty::{fen::Fen, uci::UciMove, CastlingMode, Chess, Color, Position as _, Square};

use crate::components::{BoardPerspective, ChessBoard};
use crate::puzzle::{check_puzzle_move, get_puzzle_hint, get_random_puzzle};
use crate::sound::{self, sfx};
use shared::MoveCheck;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SolveStatus {
    Solving,
    Wrong,
    Solved,
}

/// Two-stage hint, reset every time `ply` advances (a fresh move needs a
/// fresh hint) but *not* on a wrong attempt — the solver is still stuck at
/// the same ply, so an already-revealed hint should stay revealed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HintStage {
    None,
    Square,
    Arrow,
}

fn parse_fen(fen: &str) -> Option<Chess> {
    fen.parse::<Fen>()
        .ok()?
        .into_position::<Chess>(CastlingMode::Standard)
        .ok()
}

/// The vendored icon set has no dedicated `mateIn1`..`mateIn5` files (only
/// the mate *pattern* tags like `backRankMate` do) — fall back to the
/// generic `mate` icon for that whole family rather than showing nothing.
fn theme_icon(theme: &str) -> &str {
    if theme.starts_with("mateIn") {
        "mate"
    } else {
        theme
    }
}

/// Rating, which color the solver is playing, correct/incorrect feedback,
/// and — once solved — the puzzle's theme icons and a "Next puzzle" button.
/// Rendered once above the board on mobile and once beside it on desktop
/// (mirrors how `AnalysisBoard` reuses `OpeningName`/`CandidateMoves` in
/// both of its own layout blocks). The feedback line reserves its height
/// even when empty so it appearing/disappearing never shifts the
/// rating/color line.
#[component]
fn PuzzleStatusPanel(
    rating: RwSignal<i32>,
    themes: RwSignal<String>,
    solver_color: RwSignal<Option<Color>>,
    status: RwSignal<SolveStatus>,
    hint_stage: RwSignal<HintStage>,
    board_locked: RwSignal<bool>,
    #[prop(into)] on_next: Callback<()>,
    #[prop(into)] on_hint: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="rounded-md bg-zinc-900/60 border border-zinc-800 p-3 flex flex-col items-center md:items-start gap-2 text-sm text-zinc-400">
            <span>"Rating " {move || rating.get()}</span>
            // The solver's color for this puzzle, fixed once it loads — not
            // tied to `position`'s live turn, which would otherwise flicker
            // to the opponent's color for the brief window their forced
            // reply is auto-playing.
            <span class="flex items-center gap-1.5">
                <span
                    class="inline-block w-2.5 h-2.5 rounded-full border border-zinc-600"
                    class:bg-white=move || solver_color.get() == Some(Color::White)
                    class:bg-zinc-900=move || solver_color.get() == Some(Color::Black)
                ></span>
                {move || if solver_color.get() == Some(Color::Black) { "Playing Black" } else { "Playing White" }}
            </span>
            <div class="h-5 flex items-center">
                <Show when=move || status.get() == SolveStatus::Wrong>
                    <span class="px-3 py-1 rounded-md bg-red-500 text-white text-xs font-semibold shadow-lg">
                        "Try again"
                    </span>
                </Show>
                <Show when=move || status.get() == SolveStatus::Solved>
                    <span class="text-green-400 font-semibold">"Solved!"</span>
                </Show>
            </div>
            <Show when=move || status.get() != SolveStatus::Solved>
                <button
                    on:click=move |_| on_hint.run(())
                    disabled=move || board_locked.get() || hint_stage.get() == HintStage::Arrow
                    class="w-full px-3 py-1.5 rounded-md border border-zinc-700 text-xs font-medium text-zinc-300 hover:border-zinc-500 hover:text-white transition-colors cursor-pointer disabled:opacity-40 disabled:hover:border-zinc-700 disabled:hover:text-zinc-300 disabled:cursor-not-allowed"
                >
                    {move || match hint_stage.get() {
                        HintStage::None => "Hint",
                        HintStage::Square => "Show move",
                        HintStage::Arrow => "Hint used",
                    }}
                </button>
            </Show>
            <Show when=move || status.get() == SolveStatus::Solved>
                <div class="flex flex-col items-center md:items-start gap-2 pt-1 w-full">
                    <div class="flex flex-wrap justify-center md:justify-start gap-3">
                        {move || themes.get()
                            .split_whitespace()
                            .map(|theme| {
                                let theme = theme.to_string();
                                let icon = theme_icon(&theme).to_string();
                                view! {
                                    <div class="flex flex-col items-center gap-1 w-14">
                                        <img
                                            src=format!("/images/puzzle-themes/{icon}.svg")
                                            class="w-8 h-8 opacity-80"
                                            alt=theme.clone()
                                            // A few theme tags still have no vendored icon at
                                            // all (beyond the `mateIn*` family handled by
                                            // `theme_icon` above) — hide the broken-image
                                            // glyph rather than show it; the text label below
                                            // still conveys the theme either way.
                                            on:error=move |ev: leptos::ev::ErrorEvent| {
                                                use leptos::wasm_bindgen::JsCast;
                                                if let Some(el) = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok()) {
                                                    el.set_hidden(true);
                                                }
                                            }
                                        />
                                        <span class="text-[10px] text-zinc-500 text-center">{theme}</span>
                                    </div>
                                }
                            })
                            .collect_view()}
                    </div>
                    <button
                        on:click=move |_| on_next.run(())
                        class="w-full px-4 py-2 rounded-md bg-green-700 hover:bg-green-600 text-white text-sm font-semibold transition-colors"
                    >
                        "Next puzzle"
                    </button>
                </div>
            </Show>
        </div>
    }
}

#[derive(Clone)]
pub struct PuzzlesPage;

#[lazy_route]
impl LazyRoute for PuzzlesPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let load_trigger = RwSignal::new(0_u32);
        let puzzle_resource = Resource::new(
            move || load_trigger.get(),
            |_| async move { get_random_puzzle().await },
        );

        let puzzle_id = RwSignal::new(String::new());
        let rating = RwSignal::new(0_i32);
        let themes = RwSignal::new(String::new());
        let position = RwSignal::new(Chess::default());
        let last_move = RwSignal::new(None::<(Square, Square)>);
        let ply = RwSignal::new(0_usize);
        let solver_color = RwSignal::new(None::<Color>);
        let status = RwSignal::new(SolveStatus::Solving);
        let wrong_squares = RwSignal::new(Vec::<Square>::new());
        let hint_stage = RwSignal::new(HintStage::None);
        let hint_move = RwSignal::new(None::<(Square, Square)>);
        // True for the one-second beat between showing the pre-setup-move
        // position and applying the opponent's move — keeps the solver from
        // dragging a piece before there's actually something to solve.
        let board_locked = RwSignal::new(true);

        // A fresh puzzle loaded — reset all solving state, show the position
        // *before* the opponent's setup move first, then after a beat apply
        // it (and its sound) so the solver can actually see what changed,
        // rather than landing mid-puzzle with only the `last_move` highlight
        // to go on.
        Effect::new(move || {
            let Some(Ok(puzzle)) = puzzle_resource.get() else {
                return;
            };
            let Some(start_pos) = parse_fen(&puzzle.fen) else {
                return;
            };
            let Ok(setup_uci) = puzzle.first_move.parse::<UciMove>() else {
                return;
            };
            let Ok(setup_move) = setup_uci.to_move(&start_pos) else {
                return;
            };
            let Ok(solving_pos) = start_pos.clone().play(setup_move.clone()) else {
                return;
            };

            puzzle_id.set(puzzle.id.clone());
            rating.set(puzzle.rating);
            themes.set(puzzle.themes.clone());
            solver_color.set(Some(solving_pos.turn()));
            ply.set(0);
            status.set(SolveStatus::Solving);
            wrong_squares.set(vec![]);
            hint_stage.set(HintStage::None);
            hint_move.set(None);
            board_locked.set(true);
            position.set(start_pos);
            last_move.set(None);

            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(1000).await;
                sound::play(sound::for_move(&solving_pos, &setup_move));
                position.set(solving_pos);
                last_move.set(setup_move.from().map(|f| (f, setup_move.to())));
                board_locked.set(false);
            });
        });

        let next_puzzle = Callback::new(move |_: ()| load_trigger.update(|n| *n += 1));

        let on_move = Callback::new(move |m: shakmaty::Move| {
            let uci = m.to_uci(CastlingMode::Standard).to_string();
            let current_ply = ply.get_untracked();
            let id = puzzle_id.get_untracked();
            let pos = position.get_untracked();
            let Ok(new_pos) = pos.clone().play(m.clone()) else {
                return;
            };
            status.set(SolveStatus::Solving);

            leptos::task::spawn_local(async move {
                match check_puzzle_move(id, current_ply, uci).await {
                    Ok(MoveCheck::Correct { reply, solved }) => {
                        sound::play(sound::for_move(&new_pos, &m));
                        position.set(new_pos.clone());
                        last_move.set(m.from().map(|f| (f, m.to())));
                        ply.set(current_ply + 1);
                        hint_stage.set(HintStage::None);
                        hint_move.set(None);

                        if solved {
                            status.set(SolveStatus::Solved);
                            sound::play(sfx::VICTORY);
                            return;
                        }

                        if let Some(reply_uci) = reply {
                            gloo_timers::future::TimeoutFuture::new(300).await;
                            let reply_move = reply_uci
                                .parse::<UciMove>()
                                .ok()
                                .and_then(|u| u.to_move(&new_pos).ok());
                            if let Some(reply_move) = reply_move {
                                if let Ok(after_reply) = new_pos.clone().play(reply_move.clone()) {
                                    sound::play(sound::for_move(&after_reply, &reply_move));
                                    position.set(after_reply);
                                    last_move.set(reply_move.from().map(|f| (f, reply_move.to())));
                                }
                            }
                        }
                    }
                    Ok(MoveCheck::Incorrect) => {
                        status.set(SolveStatus::Wrong);
                        sound::play(sfx::ERROR);
                        let mut squares = vec![m.to()];
                        if let Some(from) = m.from() {
                            squares.push(from);
                        }
                        wrong_squares.set(squares);
                        gloo_timers::future::TimeoutFuture::new(500).await;
                        wrong_squares.set(vec![]);
                    }
                    Err(e) => {
                        leptos::logging::warn!("check_puzzle_move failed: {e}");
                    }
                }
            });
        });

        // First press fetches the correct move and reveals only its origin
        // square; a second press (same ply) reveals the full move as an
        // arrow. A third press is a no-op — `PuzzleStatusPanel` disables
        // the button once `hint_stage` reaches `Arrow`.
        let on_hint = Callback::new(move |_: ()| match hint_stage.get_untracked() {
            HintStage::None => {
                let id = puzzle_id.get_untracked();
                let current_ply = ply.get_untracked();
                leptos::task::spawn_local(async move {
                    let Ok(Some(uci)) = get_puzzle_hint(id, current_ply).await else {
                        return;
                    };
                    let pos = position.get_untracked();
                    let Some(mv) = uci.parse::<UciMove>().ok().and_then(|u| u.to_move(&pos).ok()) else {
                        return;
                    };
                    let Some(from) = mv.from() else { return };
                    hint_move.set(Some((from, mv.to())));
                    hint_stage.set(HintStage::Square);
                });
            }
            HintStage::Square => hint_stage.set(HintStage::Arrow),
            HintStage::Arrow => {}
        });

        let hint_squares = Signal::derive(move || {
            hint_move.get().map(|(from, _)| vec![from]).unwrap_or_default()
        });
        let hint_arrows = Signal::derive(move || {
            if hint_stage.get() != HintStage::Arrow {
                return vec![];
            }
            hint_move.get().map(|(from, to)| vec![(from, to, 0)]).unwrap_or_default()
        });

        let on_premove = Callback::new(move |_: (Square, Square)| {});

        let can_drag_piece = Callback::new(move |p: shakmaty::Piece| {
            !board_locked.get() && status.get() != SolveStatus::Solved && solver_color.get() == Some(p.color)
        });

        let is_my_turn = Signal::derive(|| true);

        let perspective = Signal::derive(move || match solver_color.get() {
            Some(Color::Black) => BoardPerspective::Black,
            _ => BoardPerspective::White,
        });

        // `<ChessBoard>` must mount exactly once for the page's lifetime, the
        // same way `PlayBoard`/`AnalysisBoard` mount it — its raw
        // `window_event_listener_untyped` pointer listeners (see that
        // component's own doc comment) aren't torn down on disposal, so
        // recreating the component on every "Next puzzle" click (as an
        // earlier version of this page did, by matching on the resource
        // inside a `move ||` closure and rebuilding the whole subtree) left
        // stale listeners from each old instance firing against
        // already-disposed signals — a real WASM panic in testing. Gating
        // on a `Signal` derived from the resource, rather than the
        // resource's payload itself, keeps the board mounted once `when`
        // first flips true and never rebuilds it on later emissions.
        let puzzle_loaded = Signal::derive(move || puzzle_resource.get().is_some_and(|r| r.is_ok()));
        let puzzle_error = Signal::derive(move || {
            puzzle_resource.get().and_then(|r| r.err()).map(|e| e.to_string())
        });

        view! {
            <div class="flex flex-col items-center w-full py-4 gap-4">
                <Transition fallback=|| view! { <p class="text-zinc-400">"Loading puzzle..."</p> }>
                    <Show when=move || puzzle_error.get().is_some()>
                        <p class="text-red-400">"Failed to load puzzle: " {move || puzzle_error.get().unwrap_or_default()}</p>
                    </Show>
                    <Show when=move || puzzle_loaded.get()>
                        <div class="flex flex-col items-center gap-3 w-full">
                            // Mobile: status panel stacked above the board —
                            // no room to the side on a narrow screen.
                            <div class="md:hidden w-full max-w-md">
                                <PuzzleStatusPanel
                                    rating=rating themes=themes solver_color=solver_color status=status
                                    hint_stage=hint_stage board_locked=board_locked
                                    on_next=next_puzzle on_hint=on_hint
                                />
                            </div>
                            <div class="relative w-[min(100vw,calc(100dvh-15rem))] md:w-[min(100vw,calc(100dvh-12.5rem))]">
                                <ChessBoard
                                    position={position}
                                    perspective={perspective}
                                    last_move={last_move}
                                    on_move={on_move}
                                    on_premove={on_premove}
                                    can_drag_piece={can_drag_piece}
                                    is_my_turn={is_my_turn}
                                    wrong_squares={wrong_squares}
                                    hint_squares={hint_squares}
                                    arrows={hint_arrows}
                                />
                                // Desktop: status panel beside the board,
                                // same `left-full ml-4` placement
                                // `AnalysisBoard`'s own side column uses.
                                <div class="hidden md:flex absolute top-1/2 -translate-y-1/2 left-full ml-4 w-64 flex-col gap-3">
                                    <PuzzleStatusPanel
                                        rating=rating themes=themes solver_color=solver_color status=status
                                        hint_stage=hint_stage board_locked=board_locked
                                        on_next=next_puzzle on_hint=on_hint
                                    />
                                </div>
                            </div>
                        </div>
                    </Show>
                </Transition>
            </div>
        }
        .into_any()
    }
}
