use leptos::prelude::*;
use shared::GameClientMessage;

#[derive(Clone)]
pub enum RematchState {
    Idle,
    Offering,
    OfferedToUs,
    Declined,
}

#[component]
pub fn RematchControls(
    rematch_state: RwSignal<RematchState>,
    #[prop(into)] send: Callback<GameClientMessage>,
    #[prop(optional, default = "lg")] size: &'static str,
) -> impl IntoView {
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

    let (btn_cls, text_cls) = if size == "sm" {
        (
            "px-2 py-1.5 text-base font-medium",
            "px-2 py-1.5 text-base font-medium",
        )
    } else {
        (
            "px-5 py-2.5 text-sm font-medium",
            "px-5 py-2.5 text-sm font-medium",
        )
    };

    view! {
        {move || match rematch_state.get() {
            RematchState::Idle => view! {
                <button
                    on:click=move |_| {
                        send.run(GameClientMessage::RematchOffer);
                        rematch_state.set(RematchState::Offering);
                    }
                    class=format!("{btn_cls} bg-white text-zinc-950 rounded-md hover:bg-zinc-100 transition-colors cursor-pointer whitespace-nowrap")
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
                    class=format!("{text_cls} text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer whitespace-nowrap")
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
                        class=format!("{btn_cls} bg-green-600 text-white rounded-l-md hover:bg-green-500 transition-colors cursor-pointer")
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
                        class=format!("{btn_cls} bg-red-800 text-white rounded-r-md hover:bg-red-700 transition-colors cursor-pointer")
                    >
                        "✕"
                    </button>
                </div>
            }.into_any(),
            RematchState::Declined => view! {
                <span class=format!("{text_cls} text-red-400 cursor-default")>
                    "Opponent declined"
                </span>
            }.into_any(),
        }}
    }
}
