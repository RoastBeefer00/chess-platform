use leptos::{html::Div, prelude::*};
use shakmaty::Outcome;
use shared::GameClientMessage;

#[derive(Clone)]
pub enum RematchState {
    Idle,        // initial — Rematch button shown
    Offering,    // we sent an offer, waiting for opponent — "Cancel" button
    OfferedToUs, // opponent offered — show "Accept" + "Decline"
    Declined,    // opponent declined our offer — show message briefly
}

#[component]
pub fn GameOverModal(
    outcome: Outcome,
    #[prop(into)] on_close: Callback<()>,
    #[prop(into)] on_new_game: Callback<()>,
    rematch_state: RwSignal<RematchState>,
    #[prop(into)] send: Callback<GameClientMessage>,
    tc_label: Signal<String>,
) -> impl IntoView {
    let result_text = move || match outcome {
        Outcome::Known(known_outcome) => match known_outcome {
            shakmaty::KnownOutcome::Decisive { winner } => match winner {
                shakmaty::Color::Black => "Black wins",
                shakmaty::Color::White => "White wins",
            },
            shakmaty::KnownOutcome::Draw => "Draw",
        },
        Outcome::Unknown => "this should never happen",
    };

    let card_ref = NodeRef::<Div>::new();

    #[cfg(feature = "hydrate")]
    {
        let stop = leptos_use::on_click_outside(card_ref, move |_| on_close.run(()));
        on_cleanup(stop);
    }

    #[cfg(feature = "hydrate")]
    Effect::new(move || {
        if matches!(rematch_state.get(), RematchState::Declined) {
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(2_000).await;
                if matches!(rematch_state.get_untracked(), RematchState::Declined) {
                    rematch_state.set(RematchState::Idle);
                }
            });
        }
    });

    view! {
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
            <div
                node_ref=card_ref
                class="relative w-full max-w-sm mx-4 rounded-lg bg-zinc-900 border border-zinc-800 shadow-xl p-6 text-center"
            >
                <button
                    on:click=move |_| on_close.run(())
                    class="absolute top-2 right-2 w-8 h-8 flex items-center justify-center text-zinc-400 hover:text-white rounded-md hover:bg-zinc-800 transition-colors cursor-pointer whitespace-nowrap"
                    aria-label="Close"
                >
                    "✕"
                </button>
                <h2 class="text-2xl font-semibold tracking-tight text-white mb-2">
                    {result_text}
                </h2>
                <p class="text-zinc-400 text-sm mb-6">"Game over"</p>
                <div class="flex flex-wrap items-center justify-center gap-3">
                    {move || match rematch_state.get() {
                        RematchState::Idle => view! {
                            <button
                                on:click=move |_| {
                                    send.run(GameClientMessage::RematchOffer);
                                    rematch_state.set(RematchState::Offering);
                                }
                                class="px-5 py-2.5 text-sm font-medium bg-white text-zinc-950 rounded-md hover:bg-zinc-100 transition-colors cursor-pointer whitespace-nowrap"
                            >
                                "Rematch"
                            </button>
                        }.into_any(),
                        RematchState::Offering => view! {
                            <button
                                on:click=move |_| {
                                    send.run(GameClientMessage::RematchCancel);
                                    rematch_state.set(RematchState::Idle);
                                }
                                class="px-5 py-2.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer whitespace-nowrap"
                            >
                                "Cancel"
                            </button>
                        }.into_any(),
                        RematchState::OfferedToUs => view! {
                            <div class="flex flex-row">
                                <button
                                    on:click=move |_| {
                                        send.run(GameClientMessage::RematchAccept);
                                    }
                                    title="Accept rematch"
                                    aria-label="Accept rematch"
                                    class="px-5 py-2.5 text-base font-medium bg-green-600 text-white rounded-l-md hover:bg-green-500 transition-colors cursor-pointer"
                                >
                                    "✓"
                                </button>
                                <button
                                    on:click=move |_| {
                                        send.run(GameClientMessage::RematchDecline);
                                        rematch_state.set(RematchState::Idle);
                                    }
                                    title="Decline rematch"
                                    aria-label="Decline rematch"
                                    class="px-5 py-2.5 text-base font-medium bg-red-800 text-white rounded-r-md hover:bg-red-700 transition-colors cursor-pointer"
                                >
                                    "✕"
                                </button>
                            </div>
                        }.into_any(),
                        RematchState::Declined => view! {
                            <span class="px-5 py-2.5 text-sm font-medium text-red-400 cursor-default">
                                "Opponent declined"
                            </span>
                        }.into_any(),
                    }}
                    <button
                        on:click=move |_| on_new_game.run(())
                        class="px-5 py-2.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer whitespace-nowrap"
                    >
                        {move || {
                            let label = tc_label.get();
                            if label.is_empty() {
                                "New Game".to_string()
                            } else {
                                format!("New {label}")
                            }
                        }}
                    </button>
                </div>
            </div>
        </div>
    }
}
