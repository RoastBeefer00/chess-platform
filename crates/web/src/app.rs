use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    ParamSegment, StaticSegment,
};

use crate::pages::{play::PlayPage, NotFoundPage};
use crate::{components::Nav, pages::HomePage};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body class="bg-zinc-950 text-white min-h-screen">
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/web.css"/>
        <Title text="chess-rs"/>

        <Router>
            <Nav/>
            <main class="pt-14 min-h-screen bg-zinc-950">
                <Routes fallback=NotFoundPage>
                    <Route path=StaticSegment("") view=HomePage/>
                    <Route path=(StaticSegment("play"), ParamSegment("game_id")) view=PlayPage/>
                </Routes>
            </main>
        </Router>
    }
}
