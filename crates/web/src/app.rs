use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Meta, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{ParentRoute, Route, Router, Routes},
    Lazy, ParamSegment, StaticSegment,
};

use crate::components::{Nav, Notifications, RequireAuth};
use crate::board_prefs::provide_board_prefs;
use crate::friends::provide_friends_presence;
use crate::pages::play::PlayPage;
use crate::pages::profile::ProfilePage;
use crate::pages::settings::SettingsPage;
use crate::pages::watch::WatchPage;
use crate::pages::{
    AnalysisPage, FriendsPage, HomePage, LoginPage, NotFoundPage, PuzzlesPage, StatsPage,
};
use crate::{components::auth::provide_current_user, pages::CreateUsernamePage};

/// One sentence, reused for `<meta name="description">` and the Open Graph
/// description so the two can't drift.
const SITE_DESCRIPTION: &str =
    "Play chess online — rated and casual games, tactics puzzles, and a \
     Stockfish-powered analysis board.";

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"/>
                // Browser chrome tint. Plain `<meta>` in the shell rather
                // than `<Meta>` in `App`, because `leptos_meta::Meta` has no
                // `media` attribute and these need one each. They follow the
                // system preference rather than an explicit override — a
                // `<meta>` can't read localStorage, and the only thing that
                // can diverge is the address-bar colour.
                <meta name="theme-color" content="#fafafa" media="(prefers-color-scheme: light)"/>
                <meta name="theme-color" content="#09090b" media="(prefers-color-scheme: dark)"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
                // Resolves the colour theme before first paint — see
                // `theme::THEME_BOOTSTRAP_JS` for why this can't wait for
                // hydration.
                <script inner_html=crate::theme::THEME_BOOTSTRAP_JS />
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
    provide_friends_presence();
    provide_board_prefs();

    view! {
        <Stylesheet id="leptos" href="/pkg/web.css"/>

        // Every page sets its own `<Title text="..."/>`; this formatter
        // appends the site name so they read "Watch · gambit.rs". A page
        // that sets nothing falls back to the bare site name below. This
        // matters more here than on most apps — the whole product is built
        // around having several tabs open at once (see the per-tab refcounts
        // in the friends and matchmaking sockets), and previously every one
        // of those tabs was labelled identically.
        <Title formatter=|text: String| {
            if text.is_empty() { "gambit.rs".to_string() } else { format!("{text} · gambit.rs") }
        }/>
        <Title text=""/>

        // Link previews. The analysis board and puzzles both produce
        // shareable URLs, which people paste into chat apps — without these
        // they unfurl as a bare link with no title or blurb.
        <Meta name="description" content=SITE_DESCRIPTION/>
        <Meta property="og:site_name" content="gambit.rs"/>
        <Meta property="og:type" content="website"/>
        <Meta property="og:title" content="gambit.rs"/>
        <Meta property="og:description" content=SITE_DESCRIPTION/>
        <Meta property="og:image" content="/apple-touch-icon.png"/>
        // `summary` rather than `summary_large_image`: the only art we ship
        // is a small square icon, and the large-image card would letterbox it.
        <Meta name="twitter:card" content="summary"/>


        <Router>
            <Nav/>
            <Notifications/>
            <main class="pt-nav min-h-screen bg-zinc-950 max-w-7xl mx-auto">
                <Routes fallback=NotFoundPage>
                    <Route path=StaticSegment("/") view={Lazy::<HomePage>::new()}/>
                    <Route path=StaticSegment("/analysis") view={Lazy::<AnalysisPage>::new()}/>
                    <Route path=StaticSegment("/puzzles") view={Lazy::<PuzzlesPage>::new()}/>
                    // `StaticSegment("")`, not `"/"`: this is a layout route that contributes
                    // no path segment of its own. With `"/"` the generated server-side
                    // paths came out as `//watch`, `//friends`, `//u/{username}` and so
                    // on, which Axum never matched — every protected route fell through
                    // to `file_and_error_handler`, which set 404 and did a filesystem
                    // lookup before rendering. The pages looked correct because Leptos's
                    // own router matched the real path client-side, so the only visible
                    // symptom was an HTTP 404 on a page that rendered fine.
                    <ParentRoute path=StaticSegment("") view=RequireAuth>
                        <Route path=(StaticSegment("game"), ParamSegment("game_id")) view={Lazy::<PlayPage>::new()}/>
                        <Route path=StaticSegment("watch") view={Lazy::<WatchPage>::new()}/>
                        <Route path=StaticSegment("friends") view={Lazy::<FriendsPage>::new()}/>
                        <Route path=(StaticSegment("u"), ParamSegment("username")) view={Lazy::<ProfilePage>::new()}/>
                        <Route
                            path=(StaticSegment("stats"), ParamSegment("username"), ParamSegment("category"))
                            view={Lazy::<StatsPage>::new()}
                        />
                        <Route path=StaticSegment("settings") view={Lazy::<SettingsPage>::new()}/>
                        <Route path=StaticSegment("create-username") view={Lazy::<CreateUsernamePage>::new()}/>
                    </ParentRoute>
                    <Route path=StaticSegment("/login") view={Lazy::<LoginPage>::new()}/>
                </Routes>
            </main>
        </Router>
    }
}
