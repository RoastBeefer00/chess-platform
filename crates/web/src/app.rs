use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{ParentRoute, Route, Router, Routes},
    Lazy, ParamSegment, StaticSegment,
};

use crate::components::{Nav, RequireAuth};
use crate::pages::play::PlayPage;
use crate::pages::{HomePage, LoginPage, NotFoundPage};
use crate::{components::auth::provide_current_user, pages::CreateUsernamePage};

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"/>
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
    provide_current_user();

    view! {
        <Stylesheet id="leptos" href="/pkg/web.css"/>
        <Title text="Gambit"/>

        <Router>
            <Nav/>
            <main class="pt-14 min-h-screen bg-zinc-950">
                <Routes fallback=NotFoundPage>
                    <Route path=StaticSegment("/") view={Lazy::<HomePage>::new()}/>
                    <ParentRoute path=StaticSegment("/") view=RequireAuth>
                        <Route path=(StaticSegment("game"), ParamSegment("game_id")) view={Lazy::<PlayPage>::new()}/>
                        <Route path=StaticSegment("create-username") view={Lazy::<CreateUsernamePage>::new()}/>
                    </ParentRoute>
                    <Route path=StaticSegment("/login") view={Lazy::<LoginPage>::new()}/>
                </Routes>
            </main>
        </Router>
    }
}
