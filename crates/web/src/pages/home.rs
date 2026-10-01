use leptos::prelude::*;
use leptos_router::components::Redirect;
use leptos_router::{lazy_route, LazyRoute};

use crate::components::{
    provide_my_active_game, provide_my_ratings, provide_my_recent_games, use_current_user, Landing,
    PlayHub,
};

#[derive(Clone)]
pub struct HomePage;

#[lazy_route]
impl LazyRoute for HomePage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let user = use_current_user();

        // `PlayHub` and its children are built inside the `<Transition>`
        // branch below, and that closure re-runs on every pass of
        // out-of-order streaming. A `Resource` created down there is
        // therefore a *different* resource on each pass — the same query
        // three times for one page view. Creating them here, in the route's
        // own owner, makes it once; the components pick them up by context.
        //
        // Here rather than at the App root on purpose: a resource that
        // exists is a resource Leptos drives to completion during SSR, so
        // providing these globally would fetch the signed-in user's ratings,
        // games and active game on every page, including the ones that never
        // show them. Signed-out visitors still cost nothing — the server fns
        // return "not signed in" before touching the database.
        provide_my_ratings();
        provide_my_recent_games();
        provide_my_active_game();

        view! {
            <Transition fallback=|| view! { <div class="min-h-below-nav"></div> }>
                {move || user.get().map(|res| match res {
                    // Signed-in but hasn't completed onboarding — finish that
                    // before they can do anything (otherwise they'd queue for
                    // matchmaking with username=None and their opponent would
                    // see "Anonymous").
                    Ok(Some(u)) if u.username.is_none() => {
                        view! { <Redirect path="/create-username"/> }.into_any()
                    }
                    Ok(Some(_)) => view! { <PlayHub /> }.into_any(),
                    _ => view! { <Landing /> }.into_any(),
                })}
            </Transition>
        }
        .into_any()
    }
}
