use leptos::prelude::*;
use leptos_router::{hooks::use_query_map, lazy_route, LazyRoute};
use uuid::Uuid;

use crate::components::AnalysisBoard;

#[derive(Clone)]
pub struct AnalysisPage;

#[lazy_route]
impl LazyRoute for AnalysisPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let query = use_query_map();
        let game_id =
            move || query.read().get("game").and_then(|id| Uuid::parse_str(&id).ok());

        view! {
            <AnalysisBoard game_id={Signal::derive(game_id)}/>
        }
        .into_any()
    }
}
