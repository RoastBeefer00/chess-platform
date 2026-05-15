use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{ParentRoute, Route, Router, Routes},
    Lazy, ParamSegment, StaticSegment,
};

use crate::components::{Nav, RequireAuth};
use crate::pages::{HomePage, LoginPage, NotFoundPage, RegisterPage};
use crate::pages::play::PlayPage;

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
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/web.css"/>
        <Title text="chess-rs"/>

        <Router>
            <Nav/>
            <main class="pt-14 min-h-screen bg-zinc-950">
                <Routes fallback=NotFoundPage>
                    <ParentRoute path=StaticSegment("/") view=RequireAuth>
                        <Route path=StaticSegment("") view={Lazy::<HomePage>::new()}/>
                        <Route path=(StaticSegment("play"), ParamSegment("game_id")) view={Lazy::<PlayPage>::new()}/>
                    </ParentRoute>
                    <Route path=StaticSegment("/login") view={Lazy::<LoginPage>::new()}/>
                    <Route path=StaticSegment("/register") view={Lazy::<RegisterPage>::new()}/>
                </Routes>
            </main>
        </Router>
    }
}
