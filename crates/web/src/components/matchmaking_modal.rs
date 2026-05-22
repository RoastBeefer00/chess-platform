use leptos::prelude::*;
use shared::{RatingMode, TimeControl, TimeMode};

fn format_time_control(tc: &TimeControl) -> String {
    let initial_sec = tc.initial_time / 1000;
    let inc_sec = match tc.mode {
        TimeMode::Increment(i) | TimeMode::Delay(i) => i / 1000,
    };
    let initial = if initial_sec >= 60 && initial_sec % 60 == 0 {
        format!("{}", initial_sec / 60)
    } else if initial_sec >= 60 {
        format!("{}m{}s", initial_sec / 60, initial_sec % 60)
    } else {
        format!("{}s", initial_sec)
    };
    format!("{initial}+{inc_sec}")
}

#[component]
pub fn MatchmakingModal(
    time_control: TimeControl,
    rating_mode: RatingMode,
    #[prop(into)] on_close: Callback<()>,
) -> impl IntoView {
    let elapsed = RwSignal::new(0u32);
    let tc_label = format_time_control(&time_control);
    let category = time_control.category().to_string();
    let rated_label = match rating_mode {
        RatingMode::Rated => "Rated",
        RatingMode::Casual => "Casual",
    };

    #[cfg(feature = "hydrate")]
    {
        use crate::matchmaking::matchmaking_websocket;
        use futures::channel::mpsc;
        use futures::StreamExt;
        use leptos::task::spawn_local;
        use leptos_router::NavigateOptions;
        use leptos_use::use_interval_fn;
        use shared::{MatchmakingClientMessage, MatchmakingServerMessage};

        let navigate = leptos_router::hooks::use_navigate();

        // Channel for sending messages to the server. We hold tx in a
        // StoredValue so the cleanup hook can drop it; dropping tx closes the
        // channel, which causes matchmaking_websocket to terminate server-side
        // and the spawned task to exit naturally.
        let (tx, rx) = mpsc::channel::<MatchmakingClientMessage>(1);
        let tx_holder: StoredValue<Option<mpsc::Sender<MatchmakingClientMessage>>> =
            StoredValue::new(Some(tx));

        // Send Join immediately.
        if let Some(mut tx) = tx_holder.get_value() {
            let _ = tx.try_send(MatchmakingClientMessage::Join {
                time_control: time_control.clone(),
                rating_mode,
            });
            tx_holder.set_value(Some(tx));
        }

        let nav = navigate.clone();
        spawn_local(async move {
            match matchmaking_websocket(rx.map(Ok).into()).await {
                Ok(mut messages) => {
                    while let Some(msg) = messages.next().await {
                        let Ok(msg) = msg else { continue };
                        if let MatchmakingServerMessage::Matched { game, side: _ } = msg {
                            nav(&format!("/play/{game}"), NavigateOptions::default());
                            break;
                        }
                    }
                }
                Err(e) => leptos::logging::warn!("matchmaking websocket error: {e}"),
            }
        });

        // Elapsed-seconds counter for the UI.
        use_interval_fn(
            move || elapsed.update(|v| *v += 1),
            1000,
        );

        // Drop the tx holder when the modal unmounts. Channel closes → WS ends.
        on_cleanup(move || {
            tx_holder.set_value(None);
        });
    }

    view! {
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
            <div class="w-full max-w-sm mx-4 rounded-lg bg-zinc-900 border border-zinc-800 shadow-xl p-8 text-center">
                <div class="flex justify-center mb-5">
                    <div class="w-14 h-14 border-4 border-zinc-700 border-t-white rounded-full animate-spin"></div>
                </div>
                <h2 class="text-xl font-semibold tracking-tight text-white mb-1">
                    "Searching for opponent"
                </h2>
                <div class="flex items-center justify-center gap-2 mb-1">
                    <span class="text-2xl font-bold tracking-tight text-white">{tc_label}</span>
                    <span class="text-xs font-medium uppercase tracking-wider text-zinc-500">{category}</span>
                    <span class="text-xs font-medium uppercase tracking-wider text-zinc-500">"·"</span>
                    <span class="text-xs font-medium uppercase tracking-wider text-zinc-500">{rated_label}</span>
                </div>
                <p class="text-zinc-400 text-sm mb-6">
                    {move || format!("{}s", elapsed.get())}
                </p>
                <button
                    on:click=move |_| on_close.run(())
                    class="px-5 py-2.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors"
                >
                    "Cancel"
                </button>
            </div>
        </div>
    }
}
