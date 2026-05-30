use leptos::prelude::*;

#[component]
pub fn NewGameButton(
    #[prop(into)] on_new_game: Callback<()>,
    tc_label: Signal<String>,
    #[prop(optional, default = "lg")] size: &'static str,
) -> impl IntoView {
    let cls = if size == "sm" {
        "px-2 py-1.5 text-base font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer whitespace-nowrap"
    } else {
        "px-5 py-2.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer whitespace-nowrap"
    };

    view! {
        <button on:click=move |_| on_new_game.run(()) class=cls>
            {move || {
                let label = tc_label.get();
                if label.is_empty() {
                    "New Game".to_string()
                } else {
                    format!("New {label}")
                }
            }}
        </button>
    }
}
