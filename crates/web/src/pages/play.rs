use leptos::prelude::*;
use leptos_router::{hooks::use_params_map, lazy_route, LazyRoute};
use uuid::Uuid;

use crate::components::PlayBoard;

#[derive(Clone)]
pub struct PlayPage;

#[lazy_route]
impl LazyRoute for PlayPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let params = use_params_map();
        let game_id = move || {
            params
                .read()
                .get("game_id")
                .and_then(|id| Uuid::parse_str(&id).ok())
        };

        view! {
            <div>
                {move || game_id().map(|id| view! { <PlayBoard game_id=id /> })}
            </div>
        }
        .into_any()
    }
}
