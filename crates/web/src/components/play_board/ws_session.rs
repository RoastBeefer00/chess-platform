#![cfg(feature = "hydrate")]

use futures::channel::mpsc;
use futures::StreamExt;
use leptos::prelude::*;
use shakmaty::{Outcome, Position as _, Square};
use shared::{GameClientMessage, GameServerMessage, PlayerRole};
use uuid::Uuid;

use crate::components::{DrawOfferState, RematchState};
use crate::sound::{self, sfx};

#[derive(Copy, Clone)]
pub(super) struct SessionState {
    pub position: ReadSignal<shakmaty::Chess>,
    pub set_position: WriteSignal<shakmaty::Chess>,
    pub player_role: ReadSignal<Option<PlayerRole>>,
    pub set_player_role: WriteSignal<Option<PlayerRole>>,
    pub set_move_history: WriteSignal<Vec<String>>,
    pub white_ms: RwSignal<i64>,
    pub black_ms: RwSignal<i64>,
    pub sent_at_ms: RwSignal<i64>,
    pub clock_running: RwSignal<bool>,
    pub last_move: RwSignal<Option<(Square, Square)>>,
    pub premoves: RwSignal<Vec<(Square, Square)>>,
    pub rematch_state: RwSignal<RematchState>,
    pub draw_offer_state: RwSignal<DrawOfferState>,
    pub clock_offset_ms: RwSignal<i64>,
    pub set_game_result: WriteSignal<Option<Outcome>>,
    pub on_move: Callback<shakmaty::Move>,
    pub white_wins: RwSignal<u32>,
    pub black_wins: RwSignal<u32>,
}

#[derive(Copy, Clone)]
pub(super) struct SessionHandles {
    pub current_tx: StoredValue<Option<mpsc::UnboundedSender<GameClientMessage>>, LocalStorage>,
    pub last_sent_uci: StoredValue<Option<String>, LocalStorage>,
    pub offset_samples: StoredValue<Vec<(i64, i64)>, LocalStorage>,
}

pub(super) async fn run_session(
    game_id: Uuid,
    my_uuid: Uuid,
    s: SessionState,
    h: SessionHandles,
) {
    use crate::websocket::game_websocket;
    use shared::messages::GameOverReason;
    use shakmaty::KnownOutcome;

    const HEARTBEAT_CHECK_MS: u32 = 3_000;
    const HEARTBEAT_TIMEOUT_MS: i64 = 6_000;

    let mut backoff_ms: u32 = 500;
    loop {
        let (tx, rx) = mpsc::unbounded::<GameClientMessage>();
        h.current_tx.set_value(Some(tx.clone()));
        if tx
            .unbounded_send(GameClientMessage::UserJoined { game_id })
            .is_err()
        {
            return;
        }

        match game_websocket(rx.map(Ok).into()).await {
            Ok(mut messages) => {
                backoff_ms = 500;
                let mut last_message_at = js_sys::Date::now() as i64;
                loop {
                    let heartbeat =
                        gloo_timers::future::TimeoutFuture::new(HEARTBEAT_CHECK_MS);
                    let msg = match futures::future::select(messages.next(), heartbeat).await {
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
                            white_wins,
                            black_wins,
                        } => {
                            use shakmaty::fen::Fen;
                            if let Ok(fen) = position_fen.parse::<Fen>() {
                                if let Ok(chess) = fen.into_position::<shakmaty::Chess>(
                                    shakmaty::CastlingMode::Standard,
                                ) {
                                    s.set_position.set(chess);
                                }
                            }
                            s.white_wins.set(white_wins);
                            s.black_wins.set(black_wins);
                            if uuid == my_uuid && s.player_role.get_untracked().is_none() {
                                s.set_player_role.set(Some(role));
                                s.set_move_history.set(moves);
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
                            s.white_ms.set(white_ms_left);
                            s.black_ms.set(black_ms_left);
                            s.sent_at_ms.set(server_sent_at);
                            s.clock_running.set(true);
                            s.set_move_history
                                .update(|history| history.push(uci.clone()));

                            let is_echo = h
                                .last_sent_uci
                                .with_value(|v| v.as_deref() == Some(uci.as_str()));
                            if is_echo {
                                h.last_sent_uci.set_value(None);
                                continue;
                            }

                            let applied = uci.parse::<UciMove>().ok().and_then(|u| {
                                let cur = s.position.get_untracked();
                                let m = u.to_move(&cur).ok()?;
                                let new_pos = cur.clone().play(m).ok()?;
                                Some((m, new_pos))
                            });
                            let Some((m, new_pos)) = applied else {
                                leptos::logging::warn!(
                                    "failed to apply server move {uci}; reconnecting"
                                );
                                break;
                            };
                            s.set_position.set(new_pos.clone());
                            if let Some(from) = m.from() {
                                s.last_move.set(Some((from, m.to())));
                            }
                            let sound_src = sound::for_move(&new_pos, &m);
                            leptos::logging::log!("move sound (incoming): {sound_src}");
                            sound::play(sound_src);
                            let is_my_turn = s
                                .player_role
                                .get_untracked()
                                .and_then(|r| r.color())
                                .is_some_and(|c| c == s.position.get_untracked().turn());
                            if is_my_turn {
                                let mut queue = s.premoves.get_untracked();
                                if let Some((from, to)) = queue.first().copied() {
                                    use crate::components::move_target;
                                    use shakmaty::{Position as _, Role};
                                    let legal = s.position.get_untracked().legal_moves();
                                    if let Some(mv) = legal.iter().find(|mv| {
                                        mv.from() == Some(from)
                                            && move_target(mv) == to
                                            && mv.promotion().is_none_or(|r| r == Role::Queen)
                                    }) {
                                        queue.remove(0);
                                        s.premoves.set(queue);
                                        s.on_move.run(*mv);
                                    } else {
                                        s.premoves.set(vec![]);
                                    }
                                }
                            }
                        }
                        GameServerMessage::Chat { user: _, text: _ } => {}
                        GameServerMessage::GameOver { winner, reason, white_wins, black_wins } => {
                            s.white_wins.set(white_wins);
                            s.black_wins.set(black_wins);
                            let outcome = match reason {
                                GameOverReason::Abort
                                | GameOverReason::Checkmate
                                | GameOverReason::Timeout
                                | GameOverReason::Resignation => {
                                    let w = match winner {
                                        Some(w) => w,
                                        None => {
                                            leptos::logging::warn!(
                                                "GameOver missing winner for decisive reason"
                                            );
                                            shared::Side::from(
                                                s.position.get_untracked().turn().other(),
                                            )
                                        }
                                    };
                                    Outcome::Known(KnownOutcome::Decisive {
                                        winner: w.into(),
                                    })
                                }
                                GameOverReason::Draw => Outcome::Known(KnownOutcome::Draw),
                            };
                            s.set_game_result.set(Some(outcome));
                            let my_side = s
                                .player_role
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
                            s.draw_offer_state.set(DrawOfferState::Idle);
                            s.clock_running.set(false);
                        }
                        GameServerMessage::ClockSync {
                            white_ms_left: w_ms,
                            black_ms_left: b_ms,
                            turn: _,
                            sent_at_ms: server_sent_at,
                            clock_running: running,
                        } => {
                            s.white_ms.set(w_ms);
                            s.black_ms.set(b_ms);
                            s.sent_at_ms.set(server_sent_at);
                            s.clock_running.set(running);
                        }
                        GameServerMessage::Resync {
                            position_fen,
                            moves,
                            white_ms_left: w_ms,
                            black_ms_left: b_ms,
                            turn: _,
                            sent_at_ms: server_sent_at,
                            clock_running: running,
                            white_wins,
                            black_wins,
                        } => {
                            s.white_wins.set(white_wins);
                            s.black_wins.set(black_wins);
                            use shakmaty::fen::Fen;
                            if let Ok(fen) = position_fen.parse::<Fen>() {
                                if let Ok(chess) = fen.into_position::<shakmaty::Chess>(
                                    shakmaty::CastlingMode::Standard,
                                ) {
                                    s.set_position.set(chess);
                                }
                            }
                            s.set_move_history.set(moves);
                            s.premoves.set(vec![]);
                            s.white_ms.set(w_ms);
                            s.black_ms.set(b_ms);
                            s.sent_at_ms.set(server_sent_at);
                            s.clock_running.set(running);
                        }
                        GameServerMessage::RematchOffer { from: id } => {
                            if id != my_uuid {
                                s.rematch_state.set(RematchState::OfferedToUs);
                            }
                        }
                        GameServerMessage::RematchAccept { new_game_id } => {
                            let url = format!("/game/{new_game_id}");
                            let _ = web_sys::window()
                                .and_then(|w| w.location().set_href(&url).ok());
                        }
                        GameServerMessage::RematchDecline => {
                            s.rematch_state.set(RematchState::Declined);
                        }
                        GameServerMessage::RematchCancel => {
                            s.rematch_state.set(RematchState::Idle);
                        }
                        GameServerMessage::DrawOffer { from: id } => {
                            if id != my_uuid {
                                s.draw_offer_state.set(DrawOfferState::OfferedToUs);
                            }
                        }
                        GameServerMessage::DrawDecline => {
                            s.draw_offer_state.set(DrawOfferState::Idle);
                        }
                        GameServerMessage::Pong {
                            client_time_ms,
                            server_time_ms,
                        } => {
                            let now = js_sys::Date::now() as i64;
                            let rtt = now - client_time_ms;
                            let offset = server_time_ms - (client_time_ms + rtt / 2);
                            h.offset_samples.update_value(|samples| {
                                samples.push((rtt, offset));
                                if samples.len() > 16 {
                                    samples.drain(..samples.len() - 16);
                                }
                            });
                            if let Some((_, best_offset)) = h
                                .offset_samples
                                .get_value()
                                .into_iter()
                                .min_by_key(|(rtt, _)| *rtt)
                            {
                                s.clock_offset_ms.set(best_offset);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                leptos::logging::warn!("websocket error: {e}");
            }
        }

        h.current_tx.set_value(None);
        gloo_timers::future::TimeoutFuture::new(backoff_ms).await;
        backoff_ms = (backoff_ms * 2).min(30_000);
    }
}
