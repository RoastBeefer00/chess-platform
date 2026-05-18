use crate::components::use_current_user;
use leptos::prelude::*;
use leptos_router::{lazy_route, LazyRoute, NavigateOptions};
use shared::{MatchmakingClientMessage, MatchmakingServerMessage, RatingMode};

use crate::matchmaking::matchmaking_websocket;

#[derive(Clone)]
pub struct HomePage;

#[lazy_route]
impl LazyRoute for HomePage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let navigate = leptos_router::hooks::use_navigate();

        let start_matchmaking = move || {
            use futures::channel::mpsc;
            use futures::StreamExt;
            use leptos::task::spawn_local;

            // let user = use_current_user();
            let (mut tx, rx) = mpsc::channel(1);
            let navigate = navigate.clone();

            spawn_local(async move {
                // let Some(my_uuid) = user.await.ok().flatten().map(|u| u.id) else {
                //     leptos::logging::warn!("Board mounted without authenticated user");
                //     return;
                // };
                //
                let _send_result = tx.try_send(MatchmakingClientMessage::Join {
                    bucket: shared::Bucket::Blitz180,
                    rating_mode: RatingMode::Rated,
                });

                match matchmaking_websocket(rx.map(Ok).into()).await {
                    Ok(mut messages) => {
                        while let Some(msg) = messages.next().await {
                            if let Ok(msg) = msg {
                                match msg {
                                    MatchmakingServerMessage::Queued { bucket: _ } => {}
                                    MatchmakingServerMessage::Matched { game, side: _ } => {
                                        // Redirect to game
                                        navigate(
                                            &format!("/play/{game}"),
                                            NavigateOptions::default(),
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => leptos::logging::warn!("websocket error: {e}"),
                }
            });
        };
        view! {
            <div class="flex flex-col items-center justify-center min-h-[calc(100vh-3.5rem)] px-6 text-center">
                <h1 class="text-5xl font-semibold tracking-tight text-white mb-3">
                    "Your next move"
                </h1>
                <p class="text-zinc-400 text-lg mb-8 max-w-sm">
                    "Play, learn, and improve — all in one place."
                </p>
                <div class="flex items-center gap-3">
                    <button on:click=move |_| start_matchmaking()
                       class="px-5 py-2.5 text-sm font-medium bg-white text-zinc-950 rounded-md hover:bg-zinc-100 transition-colors">
                        "Play now"
                    </button>
                    <a href="/learn"
                       class="px-5 py-2.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors">
                        "Learn"
                    </a>
                </div>
            </div>
        }
        .into_any()
    }
}
