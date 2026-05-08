use leptos::prelude::*;

use crate::components::board::Board;

#[component]
pub fn PlayPage() -> impl IntoView {
    view! {
        <div>
            <Board />
        </div>
    }
}
