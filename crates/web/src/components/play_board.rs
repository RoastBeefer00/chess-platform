use leptos::prelude::*;
use shakmaty::{Color, KnownOutcome, Outcome, Position as _};
use shared::messages::GameOverReason;
use shared::PlayerRole;
use uuid::Uuid;

use crate::components::{
    use_current_user, BoardPerspective, BoardUser, ChessBoard, Clock, GameOverModal,
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
    let searching = RwSignal::new(None::<(shared::TimeControl, shared::RatingMode)>);
    let tx_send = tx.clone();
    let send = Callback::new(move |msg: GameClientMessage| {
        let _ = tx_send.unbounded_send(msg);
    });

    let (position, set_position) = signal(shakmaty::Chess::default());
    let last_move = RwSignal::new(None::<(shakmaty::Square, shakmaty::Square)>);
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

    // on_move: gate by turn ownership, send to server via WS.
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
            let uci = m.to_uci(shakmaty::CastlingMode::Standard).to_string();
            let _ = tx.unbounded_send(GameClientMessage::MoveMade { uci });
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
                            }
                            GameServerMessage::Chat { user: _, text: _ } => {}
                            GameServerMessage::GameOver { winner, reason } => {
                                match reason {
                                    GameOverReason::Abort
                                    | GameOverReason::Checkmate
                                    | GameOverReason::Timeout => set_game_result.set(Some(
                                        Outcome::Known(KnownOutcome::Decisive {
                                            winner: winner.unwrap().into(),
                                        }),
                                    )),
                                    GameOverReason::Draw => set_game_result
                                        .set(Some(Outcome::Known(KnownOutcome::Draw))),
                                }
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
        </div>
    }
}
