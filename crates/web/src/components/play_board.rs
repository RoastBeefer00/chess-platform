use leptos::prelude::*;
use shakmaty::{Color, KnownOutcome, Outcome, Position as _};
use shared::messages::GameOverReason;
use shared::PlayerRole;
use uuid::Uuid;

#[derive(Clone, PartialEq)]
enum DrawOfferState {
    Idle,
    Offering,
    OfferedToUs,
}

use crate::components::{
    move_target, use_current_user, BoardPerspective, BoardUser, ChessBoard, Clock, GameOverModal,
    MatchmakingModal, RematchState,
};
use crate::game::get_game_info;

#[component]
pub fn PlayBoard(game_id: Uuid) -> impl IntoView {
    use futures::channel::mpsc;
    use futures::StreamExt;
    use leptos::task::spawn_local;
    use shakmaty::fen::Fen;
    use shared::{GameClientMessage, GameServerMessage};

    use crate::websocket::game_websocket;

    let user = use_current_user();

    let (tx, rx) = mpsc::unbounded::<GameClientMessage>();
    let rematch_state = RwSignal::new(RematchState::Idle);
    let draw_offer_state = RwSignal::new(DrawOfferState::Idle);
    let searching = RwSignal::new(None::<(shared::TimeControl, shared::RatingMode)>);
    let tx_send = tx.clone();
    let send = Callback::new(move |msg: GameClientMessage| {
        let _ = tx_send.unbounded_send(msg);
    });

    let (position, set_position) = signal(shakmaty::Chess::default());
    let last_move = RwSignal::new(None::<(shakmaty::Square, shakmaty::Square)>);
    let premoves = RwSignal::new(Vec::<(shakmaty::Square, shakmaty::Square)>::new());
    let (player_role, set_player_role) = signal(None::<PlayerRole>);
    let (game_result, set_game_result) = signal(None::<Outcome>);

    let white_ms = RwSignal::new(0_i64);
    let black_ms = RwSignal::new(0_i64);
    let sent_at_ms = RwSignal::new(0_i64);
    let clock_running = RwSignal::new(false);

    let perspective = Signal::derive(move || BoardPerspective::from(player_role.get()));

    // Derived per-position signals.
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
        let top_color = match perspective.get() {
            BoardPerspective::White => Color::Black,
            BoardPerspective::Black => Color::White,
        };
        turn == top_color
    });
    let bottom_active = Signal::derive(move || {
        if !clock_running.get() {
            return false;
        }
        let turn = position.get().turn();
        let bottom_color = match perspective.get() {
            BoardPerspective::White => Color::White,
            BoardPerspective::Black => Color::Black,
        };
        turn == bottom_color
    });

    // on_move: gate by turn ownership, apply optimistically, send to server via WS.
    let on_move = {
        let tx = tx.clone();
        Callback::new(move |m: shakmaty::Move| {
            use shakmaty::Position as _;
            let pos = position.get_untracked();
            let my_color = player_role.get_untracked().and_then(|r| r.color());
            if my_color != Some(pos.turn()) {
                leptos::logging::warn!("attempted move out of turn");
                return;
            }
            if let Some(from) = m.from() {
                last_move.set(Some((from, m.to())));
            }
            set_position.update(|pos| {
                if let Ok(new_pos) = pos.clone().play(m) {
                    *pos = new_pos;
                }
            });
            let uci = m.to_uci(shakmaty::CastlingMode::Standard).to_string();
            let _ = tx.unbounded_send(GameClientMessage::MoveMade { uci });
        })
    };

    let on_premove = {
        Callback::new(move |(from, to): (shakmaty::Square, shakmaty::Square)| {
            premoves.update(|premoves| premoves.push((from, to)));
        })
    };

    // can_drag_piece: only the side this player controls.
    let can_drag_piece = Callback::new(move |p: shakmaty::Piece| {
        player_role
            .get()
            .and_then(|r| r.color())
            .is_some_and(|c| c == p.color)
    });

    let tx_join = tx.clone();

    if cfg!(feature = "hydrate") {
        spawn_local(async move {
            let Some(my_uuid) = user.await.ok().flatten().map(|u| u.id) else {
                leptos::logging::warn!("PlayBoard mounted without authenticated user");
                return;
            };

            let _ = tx_join.unbounded_send(GameClientMessage::UserJoined { game_id });

            match game_websocket(rx.map(Ok).into()).await {
                Ok(mut messages) => {
                    while let Some(msg) = messages.next().await {
                        let Ok(msg) = msg else { continue };
                        match msg {
                            GameServerMessage::UserJoined {
                                uuid,
                                position_fen,
                                player_role: role,
                            } => {
                                if let Ok(fen) = position_fen.parse::<Fen>() {
                                    if let Ok(chess) = fen.into_position::<shakmaty::Chess>(
                                        shakmaty::CastlingMode::Standard,
                                    ) {
                                        set_position.set(chess);
                                    }
                                }
                                if Some(uuid) == user.await.ok().flatten().map(|u| u.id) {
                                    set_player_role.set(Some(role));
                                }
                            }
                            GameServerMessage::UserLeft { username: _ } => {}
                            GameServerMessage::MoveMade {
                                uci,
                                white_ms_left,
                                black_ms_left,
                                turn: _,
                                sent_at_ms: server_sent_at,
                            } => {
                                use shakmaty::{uci::UciMove, Position as _};
                                white_ms.set(white_ms_left);
                                black_ms.set(black_ms_left);
                                sent_at_ms.set(server_sent_at);
                                clock_running.set(true);
                                if let Ok(uci_move) = uci.parse::<UciMove>() {
                                    if let Ok(m) = uci_move.to_move(&position.get_untracked()) {
                                        if let Some(from) = m.from() {
                                            last_move.set(Some((from, m.to())));
                                        }
                                        set_position.update(|pos| {
                                            if let Ok(new_pos) = pos.clone().play(m) {
                                                *pos = new_pos;
                                            }
                                        });
                                    }
                                }
                                let is_my_turn = player_role
                                    .get_untracked()
                                    .and_then(|r| r.color())
                                    .is_some_and(|c| c == position.get_untracked().turn());
                                if is_my_turn {
                                    let mut queue = premoves.get_untracked();
                                    if let Some((from, to)) = queue.first().copied() {
                                        use shakmaty::{Position as _, Role};
                                        let legal = position.get_untracked().legal_moves();
                                        if let Some(m) = legal.iter().find(|m| {
                                            m.from() == Some(from)
                                                && move_target(m) == to
                                                && m.promotion().is_none_or(|r| r == Role::Queen)
                                        }) {
                                            queue.remove(0);
                                            premoves.set(queue);
                                            on_move.run(*m);
                                        } else {
                                            premoves.set(vec![]);
                                        }
                                    }
                                }
                            }
                            GameServerMessage::Chat { user: _, text: _ } => {}
                            GameServerMessage::GameOver { winner, reason } => {
                                match reason {
                                    GameOverReason::Abort
                                    | GameOverReason::Checkmate
                                    | GameOverReason::Timeout
                                    | GameOverReason::Resignation => set_game_result.set(Some(
                                        Outcome::Known(KnownOutcome::Decisive {
                                            winner: winner.unwrap().into(),
                                        }),
                                    )),
                                    GameOverReason::Draw => set_game_result
                                        .set(Some(Outcome::Known(KnownOutcome::Draw))),
                                }
                                draw_offer_state.set(DrawOfferState::Idle);
                                clock_running.set(false);
                            }
                            GameServerMessage::ClockSync {
                                white_ms_left: w_ms,
                                black_ms_left: b_ms,
                                turn: _,
                                sent_at_ms: server_sent_at,
                                clock_running: running,
                            } => {
                                white_ms.set(w_ms);
                                black_ms.set(b_ms);
                                sent_at_ms.set(server_sent_at);
                                clock_running.set(running);
                            }
                            GameServerMessage::RematchOffer { from: id } => {
                                if id != my_uuid {
                                    rematch_state.set(RematchState::OfferedToUs);
                                }
                            }
                            GameServerMessage::RematchAccept { new_game_id } => {
                                let url = format!("/game/{new_game_id}");
                                let _ = web_sys::window()
                                    .and_then(|w| w.location().set_href(&url).ok());
                            }
                            GameServerMessage::RematchDecline => {
                                rematch_state.set(RematchState::Declined);
                            }
                            GameServerMessage::RematchCancel => {
                                rematch_state.set(RematchState::Idle);
                            }
                            GameServerMessage::DrawOffer { from: id } => {
                                if id != my_uuid {
                                    draw_offer_state.set(DrawOfferState::OfferedToUs);
                                }
                            }
                            GameServerMessage::DrawDecline => {
                                draw_offer_state.set(DrawOfferState::Idle);
                            }
                        }
                    }
                }
                Err(e) => leptos::logging::warn!("websocket error: {e}"),
            }
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

    // Populate clock signals once the initial game info resolves.
    Effect::new(move || {
        if let Some(Ok(info)) = game_info.get() {
            white_ms.set(info.white_ms_left);
            black_ms.set(info.black_ms_left);
            sent_at_ms.set(info.sent_at_ms);
            clock_running.set(info.clock_running);
        }
    });

    provide_context(player_role);
    provide_context(premoves);

    view! {
        <div class="flex flex-col items-center justify-center w-full h-[calc(100dvh-3.5rem)]">
            <Show when=move || searching.get().is_some()>
                {move || searching.get().map(|(time_control, rating_mode)| view! {
                    <MatchmakingModal
                        time_control={time_control}
                        rating_mode={rating_mode}
                        on_close=move |_| searching.set(None)
                    />
                })}
            </Show>
            <Show when=move || game_result.get().is_some()>
                <GameOverModal
                    outcome=game_result.get().unwrap()
                    on_close=move |_| {
                        set_game_result.set(None);
                        rematch_state.set(RematchState::Idle);
                    }
                    on_new_game=move |_| {
                        if let Some(config) = game_info
                            .get_untracked()
                            .and_then(|r| r.ok())
                            .map(|info| info.config)
                        {
                            set_game_result.set(None);
                            searching.set(Some((config.time_control, config.rated)));
                        }
                    }
                    rematch_state=rematch_state
                    send=send
                    tc_label=tc_label
                />
            </Show>
            <div class="flex flex-row items-center justify-between w-[min(100vw,calc(100dvh-11.5rem))] pl-2 py-2">
                <Transition fallback=|| view! { <div class="h-12"></div> }>
                    {move || game_info.get().and_then(|res| res.ok()).map(|info| {
                        let top = match perspective.get() {
                            BoardPerspective::White => info.black.clone(),
                            BoardPerspective::Black => info.white.clone(),
                        };
                        view! { <BoardUser player={top} /> }
                    })}
                </Transition>
                <div class="flex flex-row items-center">
                    <Transition fallback=|| view! { <div></div> }>
                        {move || game_info.get().and_then(|r| r.ok()).map(|_| view! {
                            <Clock
                                snapshot_ms={top_ms}
                                snapshot_sent_at_ms={sent_at_ms.into()}
                                is_active={top_active}
                            />
                        })}
                    </Transition>
                </div>
            </div>
            <ChessBoard
                position={position}
                perspective={perspective}
                last_move={last_move}
                on_move={on_move}
                on_premove={on_premove}
                can_drag_piece={can_drag_piece}
            />
            <div class="flex flex-row items-center justify-between w-[min(100vw,calc(100dvh-11.5rem))] pl-2 py-2">
                <Transition fallback=|| view! { <div class="h-12"></div> }>
                    {move || game_info.get().and_then(|res| res.ok()).map(|info| {
                        let bottom = match perspective.get() {
                            BoardPerspective::White => info.white.clone(),
                            BoardPerspective::Black => info.black.clone(),
                        };
                        view! { <BoardUser player={bottom} /> }
                    })}
                </Transition>
                <div class="flex flex-row items-center">
                    <Transition fallback=|| view! { <div></div> }>
                        {move || game_info.get().and_then(|r| r.ok()).map(|_| view! {
                            <Clock
                                snapshot_ms={bottom_ms}
                                snapshot_sent_at_ms={sent_at_ms.into()}
                                is_active={bottom_active}
                            />
                        })}
                    </Transition>
                </div>
            </div>
            <Show when=move || {
                game_result.get().is_none()
                    && player_role.get().is_some_and(|r| matches!(r, PlayerRole::Player(_)))
            }>
                <div class="flex flex-col items-center gap-2 w-[min(100vw,calc(100dvh-11.5rem))] px-2 pb-2">
                    <Show when=move || draw_offer_state.get() == DrawOfferState::OfferedToUs>
                        <div class="flex flex-row items-center gap-3 w-full px-3 py-2 rounded-md bg-zinc-800 border border-zinc-700">
                            <span class="text-sm text-zinc-300 flex-1">"Opponent offers a draw"</span>
                            <button
                                on:click=move |_| {
                                    send.run(GameClientMessage::DrawAccept);
                                    draw_offer_state.set(DrawOfferState::Idle);
                                }
                                class="px-3 py-1 text-xs font-medium bg-green-700 text-white rounded hover:bg-green-600 transition-colors cursor-pointer"
                            >
                                "Accept"
                            </button>
                            <button
                                on:click=move |_| {
                                    send.run(GameClientMessage::DrawDecline);
                                    draw_offer_state.set(DrawOfferState::Idle);
                                }
                                class="px-3 py-1 text-xs font-medium bg-zinc-700 text-zinc-300 rounded hover:bg-zinc-600 hover:text-white transition-colors cursor-pointer"
                            >
                                "Decline"
                            </button>
                        </div>
                    </Show>
                    <div class="flex flex-row gap-2">
                        {move || match draw_offer_state.get() {
                            DrawOfferState::Idle => view! {
                                <button
                                    on:click=move |_| {
                                        send.run(GameClientMessage::DrawOffer);
                                        draw_offer_state.set(DrawOfferState::Offering);
                                    }
                                    class="px-4 py-1.5 text-xs font-medium text-zinc-300 border border-zinc-700 rounded hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                                >
                                    "Offer Draw"
                                </button>
                            }.into_any(),
                            DrawOfferState::Offering => view! {
                                <button
                                    disabled
                                    class="px-4 py-1.5 text-xs font-medium text-zinc-500 border border-zinc-800 rounded cursor-not-allowed"
                                >
                                    "Draw Offered…"
                                </button>
                            }.into_any(),
                            DrawOfferState::OfferedToUs => view! {
                                <span></span>
                            }.into_any(),
                        }}
                        <button
                            on:click=move |_| send.run(GameClientMessage::Resign)
                            class="px-4 py-1.5 text-xs font-medium text-red-400 border border-red-900 rounded hover:border-red-700 hover:text-red-300 transition-colors cursor-pointer"
                        >
                            "Resign"
                        </button>
                    </div>
                </div>
            </Show>
        </div>
    }
}
