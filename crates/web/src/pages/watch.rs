use leptos::prelude::*;
use leptos_router::{lazy_route, LazyRoute};

use crate::components::WatchGrid;

#[derive(Clone)]
pub struct WatchPage;

#[lazy_route]
impl LazyRoute for WatchPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        view! { <WatchGrid/> }.into_any()
    }
}
