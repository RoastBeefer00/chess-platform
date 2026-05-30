use leptos::prelude::*;
use shared::GameClientMessage;

#[derive(Clone, PartialEq)]
pub enum DrawOfferState {
    Idle,
    Offering,
    OfferedToUs,
}

#[component]
#[cfg_attr(not(feature = "hydrate"), allow(unused_variables))]
pub fn DrawResignControls(
    draw_offer_state: RwSignal<DrawOfferState>,
    #[prop(into)] send: Callback<GameClientMessage>,
    /// "compact" → mobile inline row (small buttons, no banner).
    /// "stacked" → desktop side column (banner + larger buttons).
    #[prop(optional, default = "compact")] variant: &'static str,
) -> impl IntoView {
    let confirming_resign = RwSignal::new(false);

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

    if variant == "stacked" {
        view! {
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
        }.into_any()
    } else {
        view! {
            <div class="flex flex-row items-center gap-1">
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
        }.into_any()
    }
}
