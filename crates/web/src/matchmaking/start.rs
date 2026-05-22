use leptos::prelude::*;
use leptos_router::NavigateOptions;
use shared::{MatchmakingClientMessage, MatchmakingServerMessage, RatingMode, TimeControl};

use crate::matchmaking::matchmaking_websocket;

/// Hook that returns a callback to start matchmaking for a given
/// `(TimeControl, RatingMode)`. The callback opens a WebSocket to the matchmaker
/// and, on a match, navigates the client to `/play/<game_id>`.
///
/// Call once in a component to get a reusable, clonable callback.
pub fn use_start_matchmaking() -> Callback<(TimeControl, RatingMode)> {
    let navigate = leptos_router::hooks::use_navigate();

    Callback::new(
        move |(time_control, rating_mode): (TimeControl, RatingMode)| {
            use futures::channel::mpsc;
            use futures::StreamExt;
            use leptos::task::spawn_local;

            let (mut tx, rx) = mpsc::channel::<MatchmakingClientMessage>(1);
            let navigate = navigate.clone();

            spawn_local(async move {
                let _ = tx.try_send(MatchmakingClientMessage::Join {
                    time_control,
                    rating_mode,
                });

                match matchmaking_websocket(rx.map(Ok).into()).await {
                    Ok(mut messages) => {
                        while let Some(msg) = messages.next().await {
                            let Ok(msg) = msg else { continue };
                            match msg {
                                MatchmakingServerMessage::Queued { time_control: _ } => {}
                                MatchmakingServerMessage::Matched { game, side: _ } => {
                                    navigate(&format!("/play/{game}"), NavigateOptions::default());
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => leptos::logging::warn!("matchmaking websocket error: {e}"),
                }
            });
        },
    )
}
