use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::{lazy_route, LazyRoute};

use crate::components::TournamentBrowser;

#[derive(Clone)]
pub struct TournamentsPage;

#[lazy_route]
impl LazyRoute for TournamentsPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        view! {
            <Title text="Tournaments"/>
            <TournamentBrowser/>
        }
        .into_any()
    }
}
