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
        "px-2 py-1.5 text-base font-medium text-zinc-300 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-white transition-colors cursor-pointer whitespace-nowrap inline-block"
    } else {
        "px-5 py-2.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-white transition-colors cursor-pointer whitespace-nowrap inline-block"
    };

    view! {
        <a href={format!("/analysis?game={game_id}")} class=cls>
            "Analyze"
        </a>
    }
}

/// The way out of a finished tournament game: back to the tournament, where
/// an arena will pair you again and a swiss shows the round. Takes the place
/// of the rematch and new-game buttons, and is styled as the primary action
/// because for a tournament player it is one.
#[component]
pub fn TournamentLink(
    tournament_id: Uuid,
    #[prop(optional, default = "lg")] size: &'static str,
) -> impl IntoView {
    let cls = if size == "sm" {
        "px-2 py-1.5 text-base font-medium bg-white text-zinc-950 rounded-control hover:bg-zinc-100 transition-colors cursor-pointer whitespace-nowrap inline-block"
    } else {
        "px-5 py-2.5 text-sm font-medium bg-white text-zinc-950 rounded-control hover:bg-zinc-100 transition-colors cursor-pointer whitespace-nowrap inline-block"
    };

    view! {
        <a href={format!("/tournament/{tournament_id}")} class=cls>
            "Back to tournament"
        </a>
    }
}
