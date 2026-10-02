//! Per-category stats, reached by pressing a rating card.
//!
//! The route carries the username as well as the category
//! (`/stats/:username/:category`) rather than reading the signed-in user
//! from the session, so the same page serves your own cards and the ones on
//! someone else's profile, and so a link to it can be shared.

use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::{hooks::use_params_map, lazy_route, LazyRoute};

use crate::components::StatsView;

#[derive(Clone)]
pub struct StatsPage;

#[lazy_route]
impl LazyRoute for StatsPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let params = use_params_map();
        // `Memo`s, not plain closures: the closure below constructs
        // `StatsView`, whose resources are created with it, and reading
        // `params` directly re-runs that on every notification the params
        // signal emits. Same reasoning as `ProfilePage` — see the note
        // there.
        let username = Memo::new(move |_| params.read().get("username").unwrap_or_default());
        let category = Memo::new(move |_| params.read().get("category").unwrap_or_default());

        view! {
            <Title text=move || format!("{} stats · {}", category.get(), username.get())/>
            <div>
                {move || {
                    let (u, c) = (username.get(), category.get());
                    (!u.is_empty() && !c.is_empty())
                        .then(|| view! { <StatsView username=u category=c/> })
                }}
            </div>
        }
        .into_any()
    }
}
