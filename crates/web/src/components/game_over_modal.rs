use leptos::{html::Div, prelude::*};
use shakmaty::Outcome;
use shared::{messages::GameOverReason, GameClientMessage};

use crate::components::{NewGameButton, RematchControls, RematchState};

#[component]
pub fn GameOverModal(
    outcome: Outcome,
    reason: Option<GameOverReason>,
    #[prop(into)] on_close: Callback<()>,
    #[prop(into)] on_new_game: Callback<()>,
    rematch_state: RwSignal<RematchState>,
    #[prop(into)] send: Callback<GameClientMessage>,
    tc_label: Signal<String>,
) -> impl IntoView {
    let is_abort = matches!(reason, Some(GameOverReason::Abort));

    let result_text = move || {
        if is_abort {
            return "Game aborted";
        }
        match outcome {
            Outcome::Known(known_outcome) => match known_outcome {
                shakmaty::KnownOutcome::Decisive { winner } => match winner {
                    shakmaty::Color::Black => "Black wins",
                    shakmaty::Color::White => "White wins",
                },
                shakmaty::KnownOutcome::Draw => "Draw",
            },
            Outcome::Unknown => "this should never happen",
        }
    };

    let subtitle = move || {
        if is_abort {
            "No rating change"
        } else {
            "Game over"
        }
    };

    let card_ref = NodeRef::<Div>::new();

    #[cfg(feature = "hydrate")]
    {
        let stop = leptos_use::on_click_outside(card_ref, move |_| on_close.run(()));
        on_cleanup(stop);
    }

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
                <p class="text-zinc-400 text-sm mb-6">{subtitle}</p>
                <div class="flex flex-wrap items-center justify-center gap-3">
                    <RematchControls rematch_state=rematch_state send=send />
                    <NewGameButton on_new_game=on_new_game tc_label=tc_label />
                </div>
            </div>
        </div>
    }
}
