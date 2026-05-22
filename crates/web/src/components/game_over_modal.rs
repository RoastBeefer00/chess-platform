use leptos::{html::Div, prelude::*};
use shakmaty::Outcome;

#[component]
pub fn GameOverModal(
    outcome: Outcome,
    #[prop(into)] on_close: Callback<()>,
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
    leptos_use::on_click_outside(card_ref, move |_| on_close.run(()));

    view! {
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
            <div
                node_ref=card_ref
                class="relative w-full max-w-sm mx-4 rounded-lg bg-zinc-900 border border-zinc-800 shadow-xl p-6 text-center"
            >
                <button
                    on:click=move |_| on_close.run(())
                    class="absolute top-2 right-2 w-8 h-8 flex items-center justify-center text-zinc-400 hover:text-white rounded-md hover:bg-zinc-800 transition-colors"
                    aria-label="Close"
                >
                    "✕"
                </button>
                <h2 class="text-2xl font-semibold tracking-tight text-white mb-2">
                    {result_text}
                </h2>
                <p class="text-zinc-400 text-sm mb-6">"Game over"</p>
                <div class="flex items-center justify-center gap-3">
                    <button class="px-5 py-2.5 text-sm font-medium bg-white text-zinc-950 rounded-md hover:bg-zinc-100 transition-colors">
                        "Rematch"
                    </button>
                    <button class="px-5 py-2.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors">
                        "New Game"
                    </button>
                </div>
            </div>
        </div>
    }
}
