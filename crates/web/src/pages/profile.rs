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
        // A `Memo`, not a plain closure: the closure below *constructs*
        // `Profile`, and `Profile`'s resources are created with it. Reading
        // `params` directly re-runs that closure on every notification the
        // params signal emits — several per SSR render — rebuilding the
        // component and re-issuing its queries each time. A memo only
        // notifies when the name actually changes, which is what should
        // rebuild the page.
        let username = Memo::new(move |_| params.read().get("username").unwrap_or_default());

        view! {
            // Resolved from the route param, so the tab is
            // labelled before the profile itself has loaded.
            <Title text=move || username.get()/>
            <div>
                {move || {
                    let username = username.get();
                    (!username.is_empty()).then(|| view! { <Profile username=username/> })
                }}
            </div>
        }
        .into_any()
    }
}
