use leptos::prelude::*;
use shakmaty::{Color, Outcome, Position as _};
use shared::PlayerRole;
use uuid::Uuid;

#[cfg(feature = "hydrate")]
mod ws_session;

use crate::components::{
    material_advantage, BoardPerspective, BoardUser, CapturedPieces, ChessBoard, Clock,
    DrawOfferState, DrawResignControls, GameOverModal, MatchmakingModal, MovesPanel, NewGameButton,
    RematchControls, RematchState,
};
use crate::game::get_game_info;
use crate::sound::{self, sfx};

fn format_score(score: f32) -> String {
    if score.fract() == 0.0 {
        format!("{}", score as u32)
    } else {
        format!("{:.1}", score)
    }
}

#[component]
#[cfg_attr(not(feature = "hydrate"), allow(unused_variables))]
pub fn PlayBoard(game_id: Uuid) -> impl IntoView {
    #[cfg(feature = "hydrate")]
    use futures::channel::mpsc;
    use shared::GameClientMessage;

    #[cfg(feature = "hydrate")]
    let current_tx: StoredValue<
        Option<mpsc::UnboundedSender<GameClientMessage>>,
        LocalStorage,
    > = StoredValue::new_local(None);
    #[cfg(feature = "hydrate")]
    let last_sent_uci: StoredValue<Option<String>, LocalStorage> = StoredValue::new_local(None);

    let rematch_state = RwSignal::new(RematchState::Idle);
    let draw_offer_state = RwSignal::new(DrawOfferState::Idle);
    let white_wins = RwSignal::new(0_f32);
    let black_wins = RwSignal::new(0_f32);
    let searching = RwSignal::new(None::<(shared::TimeControl, shared::RatingMode)>);
    // Tracks whether the GameOverModal has been dismissed. Separate from
    // `game_result` so closing the modal doesn't hide the post-game buttons.
    let modal_dismissed = RwSignal::new(false);

    let send = Callback::new(move |msg: GameClientMessage| {
        #[cfg(feature = "hydrate")]
        current_tx.with_value(|opt| {
            if let Some(tx) = opt {
                let _ = tx.unbounded_send(msg);
            }
        });
        #[cfg(not(feature = "hydrate"))]
        let _ = msg;
    });

    let (position, set_position) = signal(shakmaty::Chess::default());
    let (viewing_ply, set_viewing_ply) = signal(None::<usize>);
    let (move_history, set_move_history) = signal(Vec::<String>::new());
    let last_move = RwSignal::new(None::<(shakmaty::Square, shakmaty::Square)>);
    let premoves = RwSignal::new(Vec::<(shakmaty::Square, shakmaty::Square)>::new());
    let (player_role, set_player_role) = signal(None::<PlayerRole>);
    let (game_result, set_game_result) = signal(None::<Outcome>);
    let (end_reason, set_end_reason) = signal(None::<shared::messages::GameOverReason>);
    let abort_side = RwSignal::new(None::<shared::Side>);
    let abort_deadline_ms = RwSignal::new(None::<i64>);

    let white_ms = RwSignal::new(0_i64);
    let black_ms = RwSignal::new(0_i64);
    let sent_at_ms = RwSignal::new(0_i64);
    let clock_running = RwSignal::new(false);
    let clock_offset_ms = RwSignal::new(0_i64);
    #[cfg(feature = "hydrate")]
    let offset_samples: StoredValue<Vec<(i64, i64)>, LocalStorage> =
        StoredValue::new_local(Vec::new());

    // Clock-offset handshake: probe the server periodically with Ping.
    #[cfg(feature = "hydrate")]
    {
        use leptos_use::use_interval_fn;
        use_interval_fn(
            move || {
                send.run(GameClientMessage::Ping {
                    client_time_ms: js_sys::Date::now() as i64,
                });
            },
            3_000,
        );
        leptos::task::spawn_local(async move {
            for _ in 0..8 {
                send.run(GameClientMessage::Ping {
                    client_time_ms: js_sys::Date::now() as i64,
                });
                gloo_timers::future::TimeoutFuture::new(250).await;
            }
        });
    }

    let perspective = Signal::derive(move || BoardPerspective::from(player_role.get()));

    // Session score from each player's perspective.
    let top_wins = Signal::derive(move || match perspective.get() {
        BoardPerspective::White => black_wins.get(),
        BoardPerspective::Black => white_wins.get(),
    });
    let bottom_wins = Signal::derive(move || match perspective.get() {
        BoardPerspective::White => white_wins.get(),
        BoardPerspective::Black => black_wins.get(),
    });

    let top_advantage = Signal::derive(move || {
        let top_color = match perspective.get() {
            BoardPerspective::White => Color::Black,
            BoardPerspective::Black => Color::White,
        };
        material_advantage(&position.get(), top_color)
    });
    let bottom_advantage = Signal::derive(move || -top_advantage.get());

    let top_ms = Signal::derive(move || match perspective.get() {
        BoardPerspective::White => black_ms.get(),
        BoardPerspective::Black => white_ms.get(),
    });
    let bottom_ms = Signal::derive(move || match perspective.get() {
        BoardPerspective::White => white_ms.get(),
        BoardPerspective::Black => black_ms.get(),
    });
    let top_active = Signal::derive(move || {
        if !clock_running.get() {
            return false;
        }
        let turn = position.get().turn();
        match perspective.get() {
            BoardPerspective::White => turn == Color::Black,
            BoardPerspective::Black => turn == Color::White,
        }
    });
    let bottom_active = Signal::derive(move || {
        if !clock_running.get() {
            return false;
        }
        let turn = position.get().turn();
        match perspective.get() {
            BoardPerspective::White => turn == Color::White,
            BoardPerspective::Black => turn == Color::Black,
        }
    });

    let white_abort_deadline = Signal::derive(move || {
        if abort_side.get() == Some(shared::Side::White) {
            abort_deadline_ms.get()
        } else {
            None
        }
    });
    let black_abort_deadline = Signal::derive(move || {
        if abort_side.get() == Some(shared::Side::Black) {
            abort_deadline_ms.get()
        } else {
            None
        }
    });
    // Perspective-adjusted abort deadlines for the top/bottom clock widgets.
    let top_abort_deadline = Signal::derive(move || match perspective.get() {
        BoardPerspective::White => black_abort_deadline.get(),
        BoardPerspective::Black => white_abort_deadline.get(),
    });
    let bottom_abort_deadline = Signal::derive(move || match perspective.get() {
        BoardPerspective::White => white_abort_deadline.get(),
        BoardPerspective::Black => black_abort_deadline.get(),
    });

    let display_position = Signal::derive(move || match viewing_ply.get() {
        None => position.get(),
        Some(target_ply) => {
            use shakmaty::{uci::UciMove, Chess};
            let history = move_history.get();
            let mut pos = Chess::default();
            for uci_str in history.iter().take(target_ply) {
                if let Ok(uci) = uci_str.parse::<UciMove>() {
                    if let Ok(mv) = uci.to_move(&pos) {
                        if let Ok(next) = pos.clone().play(mv) {
                            pos = next;
                        }
                    }
                }
            }
            pos
        }
    });

    let on_move = Callback::new(move |m: shakmaty::Move| {
        use shakmaty::Position as _;
        let pos = position.get_untracked();
        let my_color = player_role.get_untracked().and_then(|r| r.color());
        if my_color != Some(pos.turn()) {
            leptos::logging::warn!("attempted move out of turn");
            return;
        }
        let Ok(new_pos) = pos.clone().play(m) else {
            leptos::logging::warn!("local play() rejected move");
            return;
        };

        #[cfg(feature = "hydrate")]
        if clock_running.get_untracked() {
            let now = js_sys::Date::now() as i64;
            let elapsed = (now - sent_at_ms.get_untracked()).max(0);
            match pos.turn() {
                Color::White => white_ms.update(|ms| *ms = (*ms - elapsed).max(0)),
                Color::Black => black_ms.update(|ms| *ms = (*ms - elapsed).max(0)),
            }
            sent_at_ms.set(now);
        }

        set_position.set(new_pos.clone());
        if let Some(from) = m.from() {
            last_move.set(Some((from, m.to())));
        }
        let sound_src = sound::for_move(&new_pos, &m);
        leptos::logging::log!("move sound (own): {sound_src}");
        sound::play(sound_src);
        let uci = m.to_uci(shakmaty::CastlingMode::Standard).to_string();
        #[cfg(feature = "hydrate")]
        last_sent_uci.set_value(Some(uci.clone()));
        let client_time_ms = {
            #[cfg(feature = "hydrate")]
            {
                js_sys::Date::now() as i64
            }
            #[cfg(not(feature = "hydrate"))]
            {
                0_i64
            }
        };
        send.run(GameClientMessage::MoveMade {
            uci,
            client_time_ms,
        });
    });

    let on_premove = Callback::new(move |(from, to): (shakmaty::Square, shakmaty::Square)| {
        premoves.update(|premoves| premoves.push((from, to)));
        sound::play(sfx::MOVE);
    });

    let can_drag_piece = Callback::new(move |p: shakmaty::Piece| {
        viewing_ply.get().is_none()
            && player_role
                .get()
                .and_then(|r| r.color())
                .is_some_and(|c| c == p.color)
    });

    // Launch the WebSocket reconnect loop.
    #[cfg(feature = "hydrate")]
    {
        use crate::components::use_current_user;
        use leptos::task::spawn_local;

        let user = use_current_user();
        let session_state = ws_session::SessionState {
            position,
            set_position,
            player_role,
            set_player_role,
            set_move_history,
            white_ms,
            black_ms,
            sent_at_ms,
            clock_running,
            last_move,
            premoves,
            rematch_state,
            draw_offer_state,
            clock_offset_ms,
            set_game_result,
            set_end_reason,
            abort_side,
            abort_deadline_ms,
            on_move,
            white_wins,
            black_wins,
        };
        let session_handles = ws_session::SessionHandles {
            current_tx,
            last_sent_uci,
            offset_samples,
        };
        spawn_local(async move {
            let Some(my_uuid) = user.await.ok().flatten().map(|u| u.id) else {
                leptos::logging::warn!("PlayBoard mounted without authenticated user");
                return;
            };
            ws_session::run_session(game_id, my_uuid, session_state, session_handles).await;
        });
    }

    let game_info = Resource::new(move || game_id, |id| async move { get_game_info(id).await });

    let tc_label = Signal::derive(move || {
        game_info
            .get()
            .and_then(|r| r.ok())
            .map(|info| {
                let tc = &info.config.time_control;
                let initial_sec = tc.initial_time / 1000;
                let inc_sec = match tc.mode {
                    shared::TimeMode::Increment(i) | shared::TimeMode::Delay(i) => i / 1000,
                };
                let initial = if initial_sec >= 60 && initial_sec % 60 == 0 {
                    format!("{}", initial_sec / 60)
                } else if initial_sec >= 60 {
                    format!("{}m{}s", initial_sec / 60, initial_sec % 60)
                } else {
                    format!("{}s", initial_sec)
                };
                format!("{initial}+{inc_sec}")
            })
            .unwrap_or_default()
    });

    let on_new_game_cb = Callback::new(move |_: ()| {
        if let Some(config) = game_info
            .get_untracked()
            .and_then(|r| r.ok())
            .map(|info| info.config)
        {
            searching.set(Some((config.time_control, config.rated)));
        }
    });

    Effect::new(move || {
        if let Some(Ok(info)) = game_info.get() {
            white_ms.set(info.white_ms_left);
            black_ms.set(info.black_ms_left);
            sent_at_ms.set(info.sent_at_ms);
            clock_running.set(info.clock_running);
        }
    });

    // Low-time warning: fire LOW_TIME sound once when my clock crosses 20s.
    #[cfg(feature = "hydrate")]
    {
        use gloo_timers::callback::Timeout;
        use leptos::prelude::{LocalStorage, StoredValue};
        let low_time_played = RwSignal::new(false);
        let low_time_timer: StoredValue<Option<Timeout>, LocalStorage> =
            StoredValue::new_local(None);
        Effect::new(move || {
            let ms = bottom_ms.get();
            let sent = sent_at_ms.get();
            let active = bottom_active.get();

            if ms > 20_000 && low_time_played.get_untracked() {
                low_time_played.set(false);
            }
            low_time_timer.set_value(None);

            if low_time_played.get_untracked() || !active || ms <= 0 {
                return;
            }
            let now = js_sys::Date::now() as i64;
            let elapsed = (now - sent).max(0);
            let remaining = ms - elapsed;
            if remaining <= 20_000 {
                sound::play(sfx::LOW_TIME);
                low_time_played.set(true);
            } else {
                let delay = (remaining - 20_000) as u32;
                let timer = Timeout::new(delay, move || {
                    sound::play(sfx::LOW_TIME);
                    low_time_played.set(true);
                });
                low_time_timer.set_value(Some(timer));
            }
        });
    }

    provide_context(player_role);
    provide_context(premoves);

    view! {
        <div class="flex flex-col items-center justify-center w-full py-2 h-[calc(100dvh-3.5rem)]">
            <Show when=move || searching.get().is_some()>
                {move || searching.get().map(|(time_control, rating_mode)| view! {
                    <MatchmakingModal
                        time_control={time_control}
                        rating_mode={rating_mode}
                        on_close=move |_| searching.set(None)
                    />
                })}
            </Show>
            <Show when=move || game_result.get().is_some() && !modal_dismissed.get()>
                <GameOverModal
                    outcome=game_result.get().unwrap()
                    reason=end_reason.get()
                    on_close=move |_| modal_dismissed.set(true)
                    on_new_game=on_new_game_cb
                    rematch_state=rematch_state
                    send=send
                    tc_label=tc_label
                />
            </Show>
            <div class="relative w-[min(100vw,calc(100dvh-15rem))] md:w-[min(100vw,calc(100dvh-12.5rem))]">
                // Top player row
                <div class="flex flex-row items-center pl-2 py-2 gap-2 overflow-hidden">
                    <div class="min-w-0 overflow-hidden">
                        <Transition fallback=|| view! { <div class="h-12"></div> }>
                            {move || game_info.get().and_then(|res| res.ok()).map(|info| {
                                let top = match perspective.get() {
                                    BoardPerspective::White => info.black.clone(),
                                    BoardPerspective::Black => info.white.clone(),
                                };
                                view! { <BoardUser player={top} /> }
                            })}
                        </Transition>
                    </div>
                    {move || view! {
                        <CapturedPieces position={position} color={match perspective.get() {
                            BoardPerspective::White => Color::White,
                            BoardPerspective::Black => Color::Black,
                        }} />
                    }}
                    {move || (top_advantage.get() > 0).then(|| view! {
                        <span class="text-xs font-semibold text-zinc-400 flex-shrink-0">
                            {format!("+{}", top_advantage.get())}
                        </span>
                    })}
                    <div class="flex flex-row items-center gap-2 flex-shrink-0 ml-auto">
                        {move || (white_wins.get() + black_wins.get() > 0.0).then(|| view! {
                            <span class="font-mono text-sm font-semibold text-zinc-300 flex-shrink-0">
                                {move || format_score(top_wins.get())}
                            </span>
                        })}
                        <Transition fallback=|| view! { <div></div> }>
                            {move || game_info.get().and_then(|r| r.ok()).map(|_| view! {
                                <Clock
                                    snapshot_ms={top_ms}
                                    snapshot_sent_at_ms={sent_at_ms.into()}
                                    is_active={top_active}
                                    offset_ms={clock_offset_ms}
                                    abort_deadline_ms={top_abort_deadline}
                                />
                            })}
                        </Transition>
                    </div>
                </div>
                <ChessBoard
                    position={display_position}
                    perspective={perspective}
                    last_move={last_move}
                    on_move={on_move}
                    on_premove={on_premove}
                    can_drag_piece={can_drag_piece}
                />
                // Bottom player row
                <div class="flex flex-row items-center pl-2 py-2 gap-2 overflow-hidden">
                    <div class="min-w-0 overflow-hidden">
                        <Transition fallback=|| view! { <div class="h-12"></div> }>
                            {move || game_info.get().and_then(|res| res.ok()).map(|info| {
                                let bottom = match perspective.get() {
                                    BoardPerspective::White => info.white.clone(),
                                    BoardPerspective::Black => info.black.clone(),
                                };
                                view! { <BoardUser player={bottom} /> }
                            })}
                        </Transition>
                    </div>
                    {move || view! {
                        <CapturedPieces position={position} color={match perspective.get() {
                            BoardPerspective::White => Color::Black,
                            BoardPerspective::Black => Color::White,
                        }} />
                    }}
                    {move || (bottom_advantage.get() > 0).then(|| view! {
                        <span class="text-xs font-semibold text-zinc-400 flex-shrink-0">
                            {format!("+{}", bottom_advantage.get())}
                        </span>
                    })}
                    <div class="flex flex-row items-center gap-2 flex-shrink-0 ml-auto">
                        // Mobile live-game controls (hidden on md+)
                        <Show when=move || {
                            game_result.get().is_none()
                                && player_role.get().is_some_and(|r| matches!(r, PlayerRole::Player(_)))
                        }>
                            <div class="md:hidden">
                                <DrawResignControls
                                    draw_offer_state=draw_offer_state
                                    send=send
                                />
                            </div>
                        </Show>
                        // Mobile post-game controls (hidden on md+)
                        <Show when=move || {
                            game_result.get().is_some()
                                && player_role.get().is_some_and(|r| matches!(r, PlayerRole::Player(_)))
                        }>
                            <div class="md:hidden flex flex-row items-center gap-1">
                                <RematchControls rematch_state=rematch_state send=send size="sm" />
                                <NewGameButton on_new_game=on_new_game_cb tc_label=tc_label size="sm" />
                            </div>
                        </Show>
                        {move || (white_wins.get() + black_wins.get() > 0.0).then(|| view! {
                            <span class="font-mono text-sm font-semibold text-zinc-300 flex-shrink-0">
                                {move || format_score(bottom_wins.get())}
                            </span>
                        })}
                        <Transition fallback=|| view! { <div></div> }>
                            {move || game_info.get().and_then(|r| r.ok()).map(|_| view! {
                                <Clock
                                    snapshot_ms={bottom_ms}
                                    snapshot_sent_at_ms={sent_at_ms.into()}
                                    is_active={bottom_active}
                                    offset_ms={clock_offset_ms}
                                    abort_deadline_ms={bottom_abort_deadline}
                                />
                            })}
                        </Transition>
                    </div>
                </div>
                // Mobile-only compact moves strip
                <div class="md:hidden px-2 pb-1">
                    <MovesPanel
                        moves={move_history}
                        viewing_ply={viewing_ply}
                        set_viewing_ply={set_viewing_ply}
                        compact=true
                    />
                </div>
                // Desktop side column: move list + controls
                <div class="hidden md:flex absolute top-0 bottom-0 left-full ml-4 w-64 flex-col gap-3">
                    <div class="flex-1 min-h-0 flex flex-col rounded-md bg-zinc-900/60 border border-zinc-800 p-2">
                        <MovesPanel moves={move_history} viewing_ply={viewing_ply} set_viewing_ply={set_viewing_ply} />
                    </div>
                    // Desktop live-game controls
                    <Show when=move || {
                        game_result.get().is_none()
                            && player_role.get().is_some_and(|r| matches!(r, PlayerRole::Player(_)))
                    }>
                        <DrawResignControls
                            draw_offer_state=draw_offer_state
                            send=send
                            variant="stacked"
                        />
                    </Show>
                    // Desktop post-game controls
                    <Show when=move || {
                        game_result.get().is_some()
                            && player_role.get().is_some_and(|r| matches!(r, PlayerRole::Player(_)))
                    }>
                        <div class="flex flex-row items-center gap-2 flex-shrink-0 flex-wrap">
                            <RematchControls rematch_state=rematch_state send=send size="sm" />
                            <NewGameButton on_new_game=on_new_game_cb tc_label=tc_label size="sm" />
                        </div>
                    </Show>
                </div>
            </div>
        </div>
    }
}
