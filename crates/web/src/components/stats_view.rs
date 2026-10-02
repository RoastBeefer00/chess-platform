//! The body of the stats page: one category's rating trend, colour split and
//! full game history.

use convert_case::{Case, Casing};
use leptos::prelude::*;
use shared::{CategoryStats, RecentGame};

use crate::components::{ColorSplit, RatingChart, RecentGameRow};

/// How many games one page of history fetches. The stats page shows a
/// category's whole history rather than the home page's ten, but an active
/// player has thousands, so it arrives a page at a time.
pub const GAMES_PAGE: usize = 50;

#[server]
pub async fn get_category_stats(
    username: String,
    category: String,
) -> Result<CategoryStats, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;
    use shared::Category;

    // Signed-in callers only, matching `get_profile` and the other
    // profile-shaped reads: this is reached from behind `RequireAuth`, and a
    // server fn is an HTTP endpoint regardless of what gates its page.
    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    if auth.user.is_none() {
        return Err(ServerFnError::new("not signed in"));
    }
    let Some(category) = Category::from_mode(&category) else {
        return Err(ServerFnError::new("unknown time control"));
    };

    let state = expect_context::<AppState>();
    let target = state
        .user_store
        .find_by_username(&username)
        .await?
        .ok_or_else(|| ServerFnError::new("user not found"))?;

    let rating_points = state.rating_store.rating_history(&target.id, category).await?;
    let (as_white, as_black) =
        state.game_store.color_record(target.id, &category.to_string()).await?;
    let current_rating = state.rating_store.get_rating(&target.id, category).await? as i32;

    Ok(CategoryStats {
        peak_rating: rating_points
            .iter()
            .map(|p| p.rating)
            .chain(std::iter::once(current_rating))
            .max(),
        rating_points,
        as_white,
        as_black,
        current_rating,
    })
}

#[server]
pub async fn get_category_games(
    username: String,
    category: String,
    offset: u32,
) -> Result<Vec<RecentGame>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;
    use shared::Category;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    if auth.user.is_none() {
        return Err(ServerFnError::new("not signed in"));
    }
    if Category::from_mode(&category).is_none() {
        return Err(ServerFnError::new("unknown time control"));
    }

    let state = expect_context::<AppState>();
    let target = state
        .user_store
        .find_by_username(&username)
        .await?
        .ok_or_else(|| ServerFnError::new("user not found"))?;

    Ok(state
        .game_store
        .list_games_in_category(target.id, &category, GAMES_PAGE as i64, offset as i64)
        .await?)
}

#[component]
pub fn StatsView(username: String, category: String) -> impl IntoView {
    let label = category.to_case(Case::Title);

    // Both resources are created here, in the route's own owner, rather than
    // inside either `<Transition>` below — a resource created inside a
    // transition branch is rebuilt on every pass of out-of-order streaming
    // and so runs its query three times per render.
    let stats = {
        let (u, c) = (username.clone(), category.clone());
        Resource::new(
            move || (u.clone(), c.clone()),
            move |(u, c)| async move { get_category_stats(u, c).await },
        )
    };

    // The history pages accumulate client-side: `offset` drives the fetch,
    // and each page is appended rather than replacing what is shown, so
    // "Load more" grows the list instead of flipping through it.
    let offset = RwSignal::new(0u32);
    let loaded = RwSignal::new(Vec::<RecentGame>::new());
    let exhausted = RwSignal::new(false);

    let page = {
        let (u, c) = (username.clone(), category.clone());
        Resource::new(
            move || (u.clone(), c.clone(), offset.get()),
            move |(u, c, off)| async move { get_category_games(u, c, off).await },
        )
    };

    Effect::new(move |_| {
        if let Some(Ok(games)) = page.get() {
            if games.len() < GAMES_PAGE {
                exhausted.set(true);
            }
            loaded.update(|all| {
                // Guard against a double-apply: an `Effect` can re-run for
                // the same resource value (a re-render, a refetch that
                // resolves to the same page), and appending twice would show
                // every game in that page twice.
                let known: std::collections::HashSet<_> = all.iter().map(|g: &RecentGame| g.id).collect();
                all.extend(games.into_iter().filter(|g| !known.contains(&g.id)));
            });
        }
    });

    let profile_href = format!("/u/{username}");

    view! {
        <div class="px-6 py-8 max-w-3xl mx-auto flex flex-col gap-8">
            <div class="flex flex-col gap-1">
                <a href={profile_href} class="eyebrow text-zinc-500 hover:text-zinc-300 transition-colors w-fit">
                    {"← "} {username.clone()}
                </a>
                <h1 class="display-1">{label.clone()}</h1>
            </div>

            <Transition fallback=|| view! {
                <div class="h-64 rounded-card skeleton-shimmer"></div>
            }>
                {move || stats.get().map(|result| match result {
                    Err(e) => view! {
                        <p class="text-red-400 text-sm">"Could not load stats: " {e.to_string()}</p>
                    }.into_any(),
                    Ok(s) => {
                        let overall = s.overall();
                        view! {
                            <div class="flex flex-col gap-8">
                                <div class="flex flex-wrap gap-8">
                                    <Figure label="Current" value={s.current_rating.to_string()}/>
                                    <Figure
                                        label="Peak"
                                        value={s.peak_rating.map(|p| p.to_string()).unwrap_or_else(|| "\u{2014}".into())}
                                    />
                                    <Figure label="Games" value={overall.total().to_string()}/>
                                    <Figure
                                        label="Score"
                                        value={overall.score_rate()
                                            .map(|r| format!("{r:.1}%"))
                                            .unwrap_or_else(|| "\u{2014}".into())}
                                    />
                                </div>
                                <section class="surface-card p-5 flex flex-col gap-4">
                                    <h2 class="eyebrow text-zinc-500">"Rating over time"</h2>
                                    <RatingChart points={s.rating_points.clone()}/>
                                </section>
                                <section class="surface-card p-5 flex flex-col gap-4">
                                    <h2 class="eyebrow text-zinc-500">"By colour"</h2>
                                    <ColorSplit white={s.as_white} black={s.as_black}/>
                                </section>
                            </div>
                        }.into_any()
                    }
                })}
            </Transition>

            <section class="flex flex-col gap-3">
                <h2 class="eyebrow text-zinc-500">{label.clone()} " games"</h2>
                <Transition fallback=|| view! {
                    <div class="h-24 rounded-card skeleton-shimmer"></div>
                }>
                    {move || {
                        let games = loaded.get();
                        if games.is_empty() {
                            return view! {
                                <p class="text-sm text-zinc-500 italic">"No games in this time control yet"</p>
                            }.into_any();
                        }
                        view! {
                            <div class="flex flex-col divide-y divide-zinc-800/70">
                                {games.into_iter()
                                    .map(|g| view! { <RecentGameRow game=g/> })
                                    .collect_view()}
                            </div>
                        }.into_any()
                    }}
                </Transition>
                <Show when=move || !exhausted.get() && !loaded.get().is_empty()>
                    <button
                        on:click=move |_| offset.update(|o| *o += GAMES_PAGE as u32)
                        class="w-full px-3 py-2 rounded-control border border-zinc-700 text-xs
                               font-medium text-zinc-300 hover:border-zinc-500 hover:text-white
                               transition-colors cursor-pointer"
                    >
                        "Load more"
                    </button>
                </Show>
            </section>
        </div>
    }
}

/// One headline number with its label.
#[component]
fn Figure(label: &'static str, value: String) -> impl IntoView {
    view! {
        <div class="flex flex-col gap-1">
            <span class="text-3xl font-bold tracking-tighter text-white leading-none">{value}</span>
            <span class="eyebrow-sm text-zinc-500">{label}</span>
        </div>
    }
}
