use leptos::prelude::*;
use uuid::Uuid;

/// Link to the analysis board pre-loaded with a finished game's moves.
/// Styled to match `NewGameButton`'s `sm`/`lg` variants so it drops into the
/// same button rows.
#[component]
pub fn AnalyzeLink(
    game_id: Uuid,
    #[prop(optional, default = "lg")] size: &'static str,
) -> impl IntoView {
    let cls = if size == "sm" {
        "px-2 py-1.5 text-base font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer whitespace-nowrap inline-block"
    } else {
        "px-5 py-2.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer whitespace-nowrap inline-block"
    };

    view! {
        <a href={format!("/analysis?game={game_id}")} class=cls>
            "Analyze"
        </a>
    }
}
