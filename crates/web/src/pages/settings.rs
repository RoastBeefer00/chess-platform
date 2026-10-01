use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::{lazy_route, LazyRoute};

use crate::components::Settings;

#[derive(Clone)]
pub struct SettingsPage;

#[lazy_route]
impl LazyRoute for SettingsPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        view! {
            <Title text="Settings"/>
            <div class="px-6 py-8 max-w-2xl mx-auto flex flex-col gap-8">
                <h1 class="display-1 text-white">"Settings"</h1>
                <Settings/>
            </div>
        }
        .into_any()
    }
}
