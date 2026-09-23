use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::{hooks::use_params_map, lazy_route, LazyRoute};

use crate::components::Profile;

#[derive(Clone)]
pub struct ProfilePage;

#[lazy_route]
impl LazyRoute for ProfilePage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let params = use_params_map();
        let username = move || params.read().get("username").unwrap_or_default();

        view! {
            // Resolved from the route param, so the tab is
            // labelled before the profile itself has loaded.
            <Title text=move || username()/>
            <div>
                {move || {
                    let username = username();
                    (!username.is_empty()).then(|| view! { <Profile username=username/> })
                }}
            </div>
        }
        .into_any()
    }
}
