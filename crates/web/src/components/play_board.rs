use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use shakmaty::KnownOutcome;
use shakmaty::{Color, Outcome, Position as _};
#[cfg(feature = "hydrate")]
use shared::messages::GameOverReason;
use shared::PlayerRole;
use uuid::Uuid;

#[derive(Clone, PartialEq)]
enum DrawOfferState {
    Idle,
    Offering,
    OfferedToUs,
}

#[cfg(feature = "hydrate")]
use crate::components::move_target;
use crate::components::{
    material_advantage, BoardPerspective, BoardUser, CapturedPieces, ChessBoard, Clock,
    GameOverModal, MatchmakingModal, MovesPanel, RematchState,
};
use crate::game::get_game_info;
use crate::sound::{self, sfx};

#[component]
#[cfg_attr(not(feature = "hydrate"), allow(unused_variables))]
pub fn PlayBoard(game_id: Uuid) -> impl IntoView {
    #[cfg(feature = "hydrate")]
    use futures::channel::mpsc;
    use shared::GameClientMessage;

    // Holds the sender for the current websocket connection. Replaced on every
    // reconnect — so `send` / `on_move` dispatch through whichever connection
    // is currently live. Mobile Safari kills backgrounded WebSockets within
    // ~30s, so this is the load-bearing piece for the reconnect loop below.
    //
    // Gated to hydrate-only because LocalStorage wraps in SendWrapper, which
    // panics on drop if dropped from a different thread than it was created
    // on. Under multi-threaded tokio SSR that fires every render.
    #[cfg(feature = "hydrate")]
    let current_tx: StoredValue<
        Option<mpsc::UnboundedSender<GameClientMessage>>,
        LocalStorage,
    > = StoredValue::new_local(None);
    // The UCI of the move we most recently sent. The server echoes every
    // applied move back via broadcast — without this, the echo of our own
    // move fails `to_move()` (piece already advanced locally) and trips the
    // desync-reconnect path on every move. Cleared once the matching echo
    // arrives.
    #[cfg(feature = "hydrate")]
    let last_sent_uci: StoredValue<Option<String>, LocalStorage> = StoredValue::new_local(None);
    let rematch_state = RwSignal::new(RematchState::Idle);
    let draw_offer_state = RwSignal::new(DrawOfferState::Idle);
    let searching = RwSignal::new(None::<(shared::TimeControl, shared::RatingMode)>);
    // First click on resign arms the button; second click within 3s sends.
    // Shared between desktop side-panel and mobile inline-row buttons.
    let confirming_resign = RwSignal::new(false);
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

    let resign_click = move |_| {
        if confirming_resign.get_untracked() {
            send.run(GameClientMessage::Resign);
            confirming_resign.set(false);
        } else {
            confirming_resign.set(true);
            #[cfg(feature = "hydrate")]
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(3_000).await;
                if confirming_resign.get_untracked() {
                    confirming_resign.set(false);
                }
            });
        }
    };

    let (position, set_position) = signal(shakmaty::Chess::default());
    let (viewing_ply, set_viewing_ply) = signal(None::<usize>);
    let (move_history, set_move_history) = signal(Vec::<String>::new());
    let last_move = RwSignal::new(None::<(shakmaty::Square, shakmaty::Square)>);
    let premoves = RwSignal::new(Vec::<(shakmaty::Square, shakmaty::Square)>::new());
    let (player_role, set_player_role) = signal(None::<PlayerRole>);
    let (game_result, set_game_result) = signal(None::<Outcome>);

    let white_ms = RwSignal::new(0_i64);
    let black_ms = RwSignal::new(0_i64);
    let sent_at_ms = RwSignal::new(0_i64);
    let clock_running = RwSignal::new(false);
    // Client->server clock offset (server ≈ client + offset), converged via the
    // ping/pong handshake below. Plain RwSignal (not LocalStorage) so it's safe
    // on SSR and readable by the Clock children.
    let clock_offset_ms = RwSignal::new(0_i64);
    // Rolling buffer of recent (rtt, offset) samples from pong replies. We apply
    // the offset of the lowest-rtt sample (least jitter = most accurate).
    // Hydrate-only: LocalStorage StoredValue panics if dropped off-thread on SSR.
    #[cfg(feature = "hydrate")]
    let offset_samples: StoredValue<Vec<(i64, i64)>, LocalStorage> =
        StoredValue::new_local(Vec::new());

    // Clock-offset handshake: probe the server periodically with Ping; the Pong
    // handler above converges `clock_offset_ms`. `send` is a no-op while
    // disconnected (current_tx is None), so this is safe across reconnects.
    #[cfg(feature = "hydrate")]
    {
        use leptos_use::use_interval_fn;
        // Steady cadence keeps the estimate fresh and tracks slow drift.
        use_interval_fn(
            move || {
                send.run(GameClientMessage::Ping {
                    client_time_ms: js_sys::Date::now() as i64,
                });
            },
            3_000,
        );
        // Initial burst for fast convergence in the first couple seconds.
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

    let top_advantage = Signal::derive(move || {
        let top_color = match perspective.get() {
            BoardPerspective::White => Color::Black,
            BoardPerspective::Black => Color::White,
        };
        material_advantage(&position.get(), top_color)
    });
    let bottom_advantage = Signal::derive(move || -top_advantage.get());

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
    let display_position = Signal::derive(move || {
        match viewing_ply.get() {
            None => position.get(), // live
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
        }
    });

    // on_move: gate by turn ownership, apply optimistically, send to server via WS.
    let on_move = Callback::new(move |m: shakmaty::Move| {
        use shakmaty::Position as _;
        let pos = position.get_untracked();
        let my_color = player_role.get_untracked().and_then(|r| r.color());
        if my_color != Some(pos.turn()) {
            leptos::logging::warn!("attempted move out of turn");
            return;
        }
        let Ok(new_pos) = pos.clone().play(m) else {
            // Local view rejected the move — don't highlight or send.
            // Server Resync will correct us if our view was stale.
            leptos::logging::warn!("local play() rejected move");
            return;
        };

        // Optimistically update the clock to match the board flip below.
        // Flipping the turn makes the opponent's clock become active; without
        // this it would re-anchor to the stale `sent_at_ms` (set when the
        // opponent last moved = start of our turn) and lurch *down* by our
        // entire think-time until the server echo arrives. We deduct our
        // think-time from our own clock and reset the anchor to now, so the
        // opponent's clock starts ticking from its full remaining instead.
        // Guarded on `clock_running`: before the first move the clocks aren't
        // running yet, and `sent_at_ms` is the join-time anchor — deducting
        // against it would wrongly drain time.
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

    let on_premove = {
        Callback::new(move |(from, to): (shakmaty::Square, shakmaty::Square)| {
            premoves.update(|premoves| premoves.push((from, to)));
            sound::play(sfx::MOVE);
        })
    };

    // can_drag_piece: only the side this player controls.
    let can_drag_piece = Callback::new(move |p: shakmaty::Piece| {
        viewing_ply.get().is_none()
            && player_role
                .get()
                .and_then(|r| r.color())
                .is_some_and(|c| c == p.color)
    });

    #[cfg(feature = "hydrate")]
    {
        use crate::components::use_current_user;
        use crate::websocket::game_websocket;
        use futures::StreamExt;
        use leptos::task::spawn_local;
        use shakmaty::fen::Fen;
        use shared::GameServerMessage;

        const HEARTBEAT_CHECK_MS: u32 = 3_000;
        const HEARTBEAT_TIMEOUT_MS: i64 = 6_000;

        let user = use_current_user();
        spawn_local(async move {
            let Some(my_uuid) = user.await.ok().flatten().map(|u| u.id) else {
                leptos::logging::warn!("PlayBoard mounted without authenticated user");
                return;
            };

            // Reconnect loop. Mobile Safari/Opera (and any backgrounded tab)
            // kills idle WebSockets; without this the user reconnects to a
            // dead WS and never sees subsequent moves or the GameOver event.
            // Each iteration creates a fresh (tx, rx) pair, parks the tx in
            // current_tx (visible to `send` / `on_move`), sends UserJoined as
            // the first message, then drains the broadcast stream until it
            // ends or errors. Exponential backoff in case the server is down.
            let mut backoff_ms: u32 = 500;
            loop {
                let (tx, rx) = mpsc::unbounded::<GameClientMessage>();
                current_tx.set_value(Some(tx.clone()));
                if tx
                    .unbounded_send(GameClientMessage::UserJoined { game_id })
                    .is_err()
                {
                    // tx dropped before we could send — unrecoverable from here.
                    return;
                }

                match game_websocket(rx.map(Ok).into()).await {
                    Ok(mut messages) => {
                        backoff_ms = 500; // reset after a successful connect
                                          // Heartbeat: a silently-dead socket (mobile handoff, NAT
                                          // timeout) leaves messages.next() Pending forever, so the
                                          // reconnect loop never fires and the board freezes. Race
                                          // each read against a timer; if no message of ANY kind
                                          // arrives within HEARTBEAT_TIMEOUT_MS, treat the socket as
                                          // a zombie and break to reconnect. The 3s Ping/Pong keeps
                                          // a healthy-but-idle connection's timestamp fresh.
                        let mut last_message_at = js_sys::Date::now() as i64;
                        loop {
                            let heartbeat =
                                gloo_timers::future::TimeoutFuture::new(HEARTBEAT_CHECK_MS);
                            let msg =
                                match futures::future::select(messages.next(), heartbeat).await {
                                    futures::future::Either::Left((msg, _)) => msg,
                                    futures::future::Either::Right((_, _)) => {
                                        if js_sys::Date::now() as i64 - last_message_at
                                            > HEARTBEAT_TIMEOUT_MS
                                        {
                                            leptos::logging::warn!(
                                                "websocket heartbeat timeout; reconnecting"
                                            );
                                            break;
                                        }
                                        continue;
                                    }
                                };
                            let Some(msg) = msg else { break };
                            let msg = match msg {
                                Ok(m) => m,
                                Err(e) => {
                                    // Stream Err (incl. BroadcastStream::Lagged) means
                                    // we may have missed messages. Bail out so the
                                    // reconnect loop pulls a fresh authoritative state.
                                    leptos::logging::warn!("ws stream error: {e}");
                                    break;
                                }
                            };
                            last_message_at = js_sys::Date::now() as i64;
                            match msg {
                                GameServerMessage::UserJoined {
                                    uuid,
                                    position_fen,
                                    player_role: role,
                                    moves,
                                } => {
                                    if let Ok(fen) = position_fen.parse::<Fen>() {
                                        if let Ok(chess) = fen.into_position::<shakmaty::Chess>(
                                            shakmaty::CastlingMode::Standard,
                                        ) {
                                            set_position.set(chess);
                                        }
                                    }
                                    if Some(uuid) == user.await.ok().flatten().map(|u| u.id)
                                        && player_role.get_untracked().is_none()
                                    {
                                        set_player_role.set(Some(role));
                                        set_move_history.set(moves);
                                        sound::play(sfx::GAME_START);
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
                                    set_move_history.update(|history| history.push(uci.clone()));

                                    // Server broadcasts every applied move to all subscribers,
                                    // including the mover. Recognise the echo of our own send
                                    // so we don't try to re-apply (position already advanced
                                    // locally) and don't replay the move sound we already
                                    // triggered in on_move.
                                    let is_echo = last_sent_uci
                                        .with_value(|v| v.as_deref() == Some(uci.as_str()));
                                    if is_echo {
                                        last_sent_uci.set_value(None);
                                        continue;
                                    }

                                    let applied = uci.parse::<UciMove>().ok().and_then(|u| {
                                        let cur = position.get_untracked();
                                        let m = u.to_move(&cur).ok()?;
                                        let new_pos = cur.clone().play(m).ok()?;
                                        Some((m, new_pos))
                                    });
                                    let Some((m, new_pos)) = applied else {
                                        // Server move didn't apply on our local position —
                                        // we've diverged. Bail to reconnect; server replays
                                        // authoritative state on UserJoined.
                                        leptos::logging::warn!(
                                            "failed to apply server move {uci}; reconnecting"
                                        );
                                        break;
                                    };
                                    set_position.set(new_pos.clone());
                                    if let Some(from) = m.from() {
                                        last_move.set(Some((from, m.to())));
                                    }
                                    let sound_src = sound::for_move(&new_pos, &m);
                                    leptos::logging::log!("move sound (incoming): {sound_src}");
                                    sound::play(sound_src);
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
                                                    && m.promotion()
                                                        .is_none_or(|r| r == Role::Queen)
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
                                    let outcome = match reason {
                                        GameOverReason::Abort
                                        | GameOverReason::Checkmate
                                        | GameOverReason::Timeout
                                        | GameOverReason::Resignation => {
                                            // Defensive: a decisive reason without a winner is a
                                            // server bug. Fall back to the side opposite the
                                            // current turn so the modal still renders something
                                            // sensible instead of panicking the whole WS loop.
                                            let w = match winner {
                                                Some(w) => w,
                                                None => {
                                                    leptos::logging::warn!(
                                                    "GameOver missing winner for decisive reason"
                                                );
                                                    shared::Side::from(
                                                        position.get_untracked().turn().other(),
                                                    )
                                                }
                                            };
                                            Outcome::Known(KnownOutcome::Decisive {
                                                winner: w.into(),
                                            })
                                        }
                                        GameOverReason::Draw => Outcome::Known(KnownOutcome::Draw),
                                    };
                                    set_game_result.set(Some(outcome));
                                    let my_side = player_role
                                        .get_untracked()
                                        .and_then(|r| r.color())
                                        .map(shared::Side::from);
                                    let sound_src = match (winner, my_side) {
                                        (None, _) => sfx::DRAW,
                                        (Some(w), Some(mine)) if w == mine => sfx::VICTORY,
                                        (Some(_), Some(_)) => sfx::DEFEAT,
                                        (Some(_), None) => sfx::MOVE,
                                    };
                                    sound::play(sound_src);
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
                                GameServerMessage::Resync {
                                    position_fen,
                                    moves,
                                    white_ms_left: w_ms,
                                    black_ms_left: b_ms,
                                    turn: _,
                                    sent_at_ms: server_sent_at,
                                    clock_running: running,
                                } => {
                                    // Authoritative snapshot — server detected a desync
                                    // (rejected our move, or broadcast lag). Replace
                                    // local state and drop any optimistic premoves so
                                    // the board stops diverging.
                                    if let Ok(fen) = position_fen.parse::<Fen>() {
                                        if let Ok(chess) = fen.into_position::<shakmaty::Chess>(
                                            shakmaty::CastlingMode::Standard,
                                        ) {
                                            set_position.set(chess);
                                        }
                                    }
                                    set_move_history.set(moves);
                                    premoves.set(vec![]);
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
                                #[cfg(feature = "hydrate")]
                                GameServerMessage::Pong {
                                    client_time_ms,
                                    server_time_ms,
                                } => {
                                    let now = js_sys::Date::now() as i64;
                                    let rtt = now - client_time_ms;
                                    // NTP-style: server time at midpoint of the round-trip
                                    let offset = server_time_ms - (client_time_ms + rtt / 2);
                                    offset_samples.update_value(|samples| {
                                        samples.push((rtt, offset));
                                        // Keep only the 16 most recent samples
                                        if samples.len() > 16 {
                                            samples.drain(..samples.len() - 16);
                                        }
                                    });
                                    // Best estimate = offset from the lowest-RTT sample (least jitter)
                                    if let Some((_, best_offset)) = offset_samples
                                        .get_value()
                                        .into_iter()
                                        .min_by_key(|(rtt, _)| *rtt)
                                    {
                                        clock_offset_ms.set(best_offset);
                                    }
                                }
                                #[cfg(not(feature = "hydrate"))]
                                GameServerMessage::Pong { .. } => {}
                            }
                        }
                    }
                    Err(e) => {
                        leptos::logging::warn!("websocket error: {e}");
                    }
                }

                // The connection ended. Drop our reference to the dead tx so
                // any callbacks that fire mid-disconnect become silent no-ops
                // instead of pushing into a void.
                current_tx.set_value(None);
                gloo_timers::future::TimeoutFuture::new(backoff_ms).await;
                backoff_ms = (backoff_ms * 2).min(30_000);
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

    // Low-time warning: fire LOW_TIME sound once when my clock crosses 20s.
    // Snapshot only updates on server pushes, so use a timer for accuracy.
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

            // Reset the flag if clock returned above threshold (e.g. after increment).
            if ms > 20_000 && low_time_played.get_untracked() {
                low_time_played.set(false);
            }
            // Cancel previous timer; we'll re-schedule if appropriate.
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
            <div class="relative w-[min(100vw,calc(100dvh-15rem))] md:w-[min(100vw,calc(100dvh-12.5rem))]">
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
                    <div class="flex flex-row items-center flex-shrink-0 ml-auto">
                        <Transition fallback=|| view! { <div></div> }>
                            {move || game_info.get().and_then(|r| r.ok()).map(|_| view! {
                                <Clock
                                    snapshot_ms={top_ms}
                                    snapshot_sent_at_ms={sent_at_ms.into()}
                                    is_active={top_active}
                                    offset_ms={clock_offset_ms}
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
                        <Show when=move || {
                            game_result.get().is_none()
                                && player_role.get().is_some_and(|r| matches!(r, PlayerRole::Player(_)))
                        }>
                            <div class="md:hidden flex flex-row items-center gap-1">
                                {move || match draw_offer_state.get() {
                                    DrawOfferState::Idle => view! {
                                        <button
                                            on:click=move |_| {
                                                send.run(GameClientMessage::DrawOffer);
                                                draw_offer_state.set(DrawOfferState::Offering);
                                            }
                                            title="Offer Draw"
                                            class="px-2 py-1 text-xs font-medium text-zinc-300 border border-zinc-700 rounded hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                                        >
                                            "½"
                                        </button>
                                    }.into_any(),
                                    DrawOfferState::Offering => view! {
                                        <button
                                            disabled
                                            title="Draw offered"
                                            class="px-2 py-1 text-xs font-medium text-zinc-500 border border-zinc-800 rounded cursor-not-allowed"
                                        >
                                            "½…"
                                        </button>
                                    }.into_any(),
                                    DrawOfferState::OfferedToUs => view! {
                                        <button
                                            on:click=move |_| {
                                                send.run(GameClientMessage::DrawAccept);
                                                draw_offer_state.set(DrawOfferState::Idle);
                                            }
                                            title="Accept draw"
                                            class="px-2 py-1 text-xs font-medium bg-green-700 text-white rounded hover:bg-green-600 transition-colors cursor-pointer"
                                        >
                                            "✓½"
                                        </button>
                                    }.into_any(),
                                }}
                                <Show when=move || draw_offer_state.get() == DrawOfferState::OfferedToUs>
                                    <button
                                        on:click=move |_| {
                                            send.run(GameClientMessage::DrawDecline);
                                            draw_offer_state.set(DrawOfferState::Idle);
                                        }
                                        title="Decline draw"
                                        class="px-2 py-1 text-xs font-medium text-zinc-300 border border-zinc-700 rounded hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                                    >
                                        "✕"
                                    </button>
                                </Show>
                                <button
                                    on:click=resign_click
                                    title=move || if confirming_resign.get() { "Click again to confirm" } else { "Resign" }
                                    class="px-2 py-1 text-xs font-medium border rounded transition-colors cursor-pointer whitespace-nowrap"
                                    class:text-red-400=move || !confirming_resign.get()
                                    class:border-red-900=move || !confirming_resign.get()
                                    class:hover:border-red-700=move || !confirming_resign.get()
                                    class:hover:text-red-300=move || !confirming_resign.get()
                                    class:bg-red-700=move || confirming_resign.get()
                                    class:border-red-700=move || confirming_resign.get()
                                    class:text-white=move || confirming_resign.get()
                                >
                                    {move || if confirming_resign.get() { "Sure?" } else { "⚑" }}
                                </button>
                            </div>
                        </Show>
                        <Transition fallback=|| view! { <div></div> }>
                            {move || game_info.get().and_then(|r| r.ok()).map(|_| view! {
                                <Clock
                                    snapshot_ms={bottom_ms}
                                    snapshot_sent_at_ms={sent_at_ms.into()}
                                    is_active={bottom_active}
                                    offset_ms={clock_offset_ms}
                                />
                            })}
                        </Transition>
                    </div>
                </div>
                // Mobile-only compact moves strip below the bottom clock.
                // Single horizontal scrolling row with nav buttons at each end.
                <div class="md:hidden px-2 pb-1">
                    <MovesPanel
                        moves={move_history}
                        viewing_ply={viewing_ply}
                        set_viewing_ply={set_viewing_ply}
                        compact=true
                    />
                </div>
                // Side column (desktop+): scrolling move list on top, live-game
                // draw/resign controls beneath. Anchored to the board's full
                // height so the move list takes whatever vertical space is left
                // after the buttons.
                <div class="hidden md:flex absolute top-0 bottom-0 left-full ml-4 w-64 flex-col gap-3">
                    <div class="flex-1 min-h-0 flex flex-col rounded-md bg-zinc-900/60 border border-zinc-800 p-2">
                        <MovesPanel moves={move_history} viewing_ply={viewing_ply} set_viewing_ply={set_viewing_ply} />
                    </div>
                    <Show when=move || {
                        game_result.get().is_none()
                            && player_role.get().is_some_and(|r| matches!(r, PlayerRole::Player(_)))
                    }>
                        <div class="flex flex-col gap-2 flex-shrink-0">
                            <Show when=move || draw_offer_state.get() == DrawOfferState::OfferedToUs>
                                <div class="flex flex-col items-stretch gap-2 px-3 py-2 rounded-md bg-zinc-800 border border-zinc-700">
                                    <span class="text-sm text-zinc-300 whitespace-nowrap">"Opponent offers a draw"</span>
                                    <div class="flex flex-row gap-2">
                                        <button
                                            on:click=move |_| {
                                                send.run(GameClientMessage::DrawAccept);
                                                draw_offer_state.set(DrawOfferState::Idle);
                                            }
                                            title="Accept draw"
                                            aria-label="Accept draw"
                                            class="flex-1 px-3 py-1 text-base font-medium bg-green-700 text-white rounded hover:bg-green-600 transition-colors cursor-pointer"
                                        >
                                            "✓"
                                        </button>
                                        <button
                                            on:click=move |_| {
                                                send.run(GameClientMessage::DrawDecline);
                                                draw_offer_state.set(DrawOfferState::Idle);
                                            }
                                            title="Decline draw"
                                            aria-label="Decline draw"
                                            class="flex-1 px-3 py-1 text-base font-medium bg-zinc-700 text-zinc-300 rounded hover:bg-zinc-600 hover:text-white transition-colors cursor-pointer"
                                        >
                                            "✕"
                                        </button>
                                    </div>
                                </div>
                            </Show>
                            <div class="flex flex-row items-stretch gap-1">
                                {move || match draw_offer_state.get() {
                                    DrawOfferState::Idle => view! {
                                        <button
                                            on:click=move |_| {
                                                send.run(GameClientMessage::DrawOffer);
                                                draw_offer_state.set(DrawOfferState::Offering);
                                            }
                                            title="Offer draw"
                                            aria-label="Offer draw"
                                            class="flex-1 px-2 py-1.5 text-base font-medium text-zinc-300 border border-zinc-700 rounded hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                                        >
                                            "½"
                                        </button>
                                    }.into_any(),
                                    DrawOfferState::Offering => view! {
                                        <button
                                            disabled
                                            title="Draw offered"
                                            aria-label="Draw offered"
                                            class="flex-1 px-2 py-1.5 text-base font-medium text-zinc-500 border border-zinc-800 rounded cursor-not-allowed"
                                        >
                                            "½…"
                                        </button>
                                    }.into_any(),
                                    DrawOfferState::OfferedToUs => view! {
                                        <div class="flex-1"></div>
                                    }.into_any(),
                                }}
                                <button
                                    on:click=resign_click
                                    title=move || if confirming_resign.get() { "Click again to confirm" } else { "Resign" }
                                    aria-label=move || if confirming_resign.get() { "Confirm resign" } else { "Resign" }
                                    class="flex-1 px-2 py-1.5 text-base font-medium border rounded transition-colors cursor-pointer whitespace-nowrap"
                                    class:text-red-400=move || !confirming_resign.get()
                                    class:border-red-900=move || !confirming_resign.get()
                                    class:hover:border-red-700=move || !confirming_resign.get()
                                    class:hover:text-red-300=move || !confirming_resign.get()
                                    class:bg-red-700=move || confirming_resign.get()
                                    class:border-red-700=move || confirming_resign.get()
                                    class:text-white=move || confirming_resign.get()
                                >
                                    {move || if confirming_resign.get() { "Sure?" } else { "⚑" }}
                                </button>
                            </div>
                        </div>
                    </Show>
                </div>
            </div>
        </div>
    }
}
