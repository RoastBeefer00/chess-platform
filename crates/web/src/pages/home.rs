use leptos::prelude::*;
use leptos_router::{lazy_route, LazyRoute};

use crate::components::{use_current_user, Landing, PlayHub};

#[derive(Clone)]
pub struct HomePage;

#[lazy_route]
impl LazyRoute for HomePage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let user = use_current_user();

        view! {
            <Transition fallback=|| view! { <div class="min-h-[calc(100dvh-3.5rem)]"></div> }>
                {move || user.get().map(|res| match res {
                    Ok(Some(_)) => view! { <PlayHub /> }.into_any(),
                    _ => view! { <Landing /> }.into_any(),
                })}
            </Transition>
        }
        .into_any()
    }
}
