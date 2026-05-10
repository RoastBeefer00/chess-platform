use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use uuid::Uuid;

use crate::components::board::Board;

#[component]
pub fn PlayPage() -> impl IntoView {
    let params = use_params_map();
    let game_id = move || {
        params
            .read()
            .get("game_id")
            .and_then(|id| Uuid::parse_str(&id).ok())
    };

    view! {
        <div>
            {move || game_id().map(|id| view! { <Board game_id=id /> })}
        </div>
    }
}
