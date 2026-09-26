use convert_case::{Case, Casing};
use leptos::prelude::*;
use shared::Category;
use strum::IntoEnumIterator;

/// Every category's rating and 14-day diff for one user, in one round trip.
///
/// `username` of `None` means the signed-in caller, resolved server-side in
/// the same call — so `EloCardRow` takes a plain `String` prop and never has
/// to wait on another resource to resolve first. Nesting a fresh `Resource`
/// inside another resource's already-resolved branch (as the profile page
/// originally did, keying off the profile fetch's returned user id)
/// intermittently desynced SSR and hydration for sibling resources.
///
/// This replaced a per-category server fn that the caller invoked once for
/// each of the four categories. Leptos resolves those concurrently during
/// SSR, so a single page render opened four connections for what is one
/// table, one user, and one query.
#[server]
pub async fn get_all_ratings(
    username: Option<String>,
) -> Result<Ratings, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let app_state = expect_context::<AppState>();
    let id = match username {
        Some(name) => app_state
            .user_store
            .find_by_username(&name)
            .await?
            .ok_or_else(|| ServerFnError::new("user not found"))?
            .id,
        None => {
            let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
            auth.user.as_ref().map(|u| u.id).ok_or_else(|| ServerFnError::new("not signed in"))?
        }
    };
    app_state
        .rating_store
        .get_all_ratings_with_diff(&id)
        .await
        .map_err(|e| {
            tracing::error!(%id, error = %e, "get_all_ratings failed");
            ServerFnError::new(format!("Error getting ratings for user {id}: {e}"))
        })
}

/// The signed-in user's own ratings, fetched once per page load.
///
/// This lives at the App root rather than inside `EloCardRow` because of how
/// `Resource::new` binds to the reactive owner that is current when it runs.
/// `EloCardRow` is constructed inside `HomePage`'s `<Transition>` branch, and
/// that closure re-runs on every pass of out-of-order streaming — three
/// passes per SSR render. A resource created in the component body is
/// therefore a *different* resource on each pass, and the query runs three
/// times for one page view. Created once at the root, it runs once.
/// One row per category: the rating and its 14-day change.
pub type Ratings = Vec<(Category, u32, i32)>;

#[derive(Copy, Clone)]
pub struct MyRatingsResource(pub Resource<Result<Ratings, ServerFnError>>);

/// Call once at the App root.
pub fn provide_my_ratings() {
    provide_context(MyRatingsResource(Resource::new(
        || (),
        |_| async move { get_all_ratings(None).await },
    )));
}

pub fn use_my_ratings() -> Resource<Result<Ratings, ServerFnError>> {
    use_context::<MyRatingsResource>()
        .expect("provide_my_ratings must be called at the App root")
        .0
}

fn category_icon(category: Category) -> AnyView {
    let color = match category {
        Category::Bullet => "text-orange-400/75",
        Category::Blitz => "text-amber-400/75",
        Category::Rapid => "text-sky-400/75",
        Category::Classical => "text-violet-400/75",
    };
    match category {
        Category::Bullet => view! {
            <svg class=color width="15" height="15" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z"/>
            </svg>
        }.into_any(),
        Category::Blitz => view! {
            <svg class=color width="15" height="15" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>
            </svg>
        }.into_any(),
        Category::Rapid => view! {
            <svg class=color width="15" height="15" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="10"/>
                <polyline points="12 6 12 12 16 14"/>
            </svg>
        }.into_any(),
        Category::Classical => view! {
            <svg class=color width="15" height="15" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <path d="M5 22h14M5 2h14M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22M7 2v4.172a2 2 0 0 1 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/>
            </svg>
        }.into_any(),
    }
}

/// Backoff schedule for retrying a failed rating fetch. Sized to ride out a
/// Fly autostop reboot (machine stops on idle, cold-starts on the next
/// request — see `fly.toml`'s `auto_stop_machines`): every in-flight request
/// during that window dies, and without a retry the row would otherwise
/// show "\u{2014}" forever until the user manually reloads.
#[cfg(feature = "hydrate")]
const RATING_RETRY_BACKOFF_MS: [u32; 4] = [1_000, 2_000, 4_000, 8_000];

/// The strip of rating cards, one per category.
///
/// Owns the single fetch for all four — the cards themselves are
/// presentational. Previously each card owned its own `Resource`, so a row
/// of four meant four server-fn calls resolved concurrently during SSR, and
/// four simultaneous pool connections for one user's ratings.
///
/// `username` of `None` renders the signed-in user's own ratings.
#[component]
pub fn EloCardRow(#[prop(optional, into)] username: Option<String>) -> impl IntoView {
    // The signed-in user's own row comes from the root-owned resource (see
    // `provide_my_ratings`); a named profile is a different user's ratings,
    // and the profile page constructs this outside any resource-gated
    // branch, so a local resource there runs once.
    let ratings = match username {
        None => use_my_ratings(),
        Some(name) => Resource::new(
            move || name.clone(),
            move |name| async move { get_all_ratings(Some(name)).await },
        ),
    };

    #[cfg(feature = "hydrate")]
    {
        let retry_count = RwSignal::new(0usize);
        Effect::new(move |_| match ratings.get() {
            Some(Err(_)) => {
                let attempt = retry_count.get_untracked();
                if let Some(backoff) = RATING_RETRY_BACKOFF_MS.get(attempt).copied() {
                    retry_count.set(attempt + 1);
                    leptos::task::spawn_local(async move {
                        gloo_timers::future::TimeoutFuture::new(backoff).await;
                        ratings.refetch();
                    });
                }
            }
            Some(Ok(_)) => retry_count.set(0),
            None => {}
        });
    }

    // One `Transition` around the whole strip rather than one per card, so
    // the four resolve together instead of popping in independently.
    view! {
        <Transition fallback=|| view! {
            <>
                {Category::iter().map(|c| view! { <EloCardSkeleton category=c/> }).collect_view()}
            </>
        }>
            {move || {
                let found = ratings.get().and_then(|r| r.ok()).unwrap_or_default();
                Category::iter()
                    .map(|category| {
                        let value = found
                            .iter()
                            .find(|(c, _, _)| c.to_string() == category.to_string())
                            .map(|(_, rating, diff)| (*rating, *diff));
                        view! { <EloCard category=category value=value/> }
                    })
                    .collect_view()
            }}
        </Transition>
    }
}

/// Placeholder with the card's exact footprint, so the strip doesn't reflow
/// when the real values land.
#[component]
fn EloCardSkeleton(category: Category) -> impl IntoView {
    let _ = category;
    view! {
        <div class="flex flex-col gap-3 p-5 surface-card min-w-[128px]">
            <div class="w-4 h-4 rounded skeleton-shimmer"/>
            <div class="flex flex-col gap-2">
                <div class="w-16 h-7 rounded skeleton-shimmer"/>
                <div class="w-8 h-2.5 rounded skeleton-shimmer"/>
                <div class="w-10 h-2 rounded skeleton-shimmer"/>
            </div>
        </div>
    }
}

/// A single rating card. Purely presentational — `EloCardRow` above does the
/// fetching. `value` of `None` renders the em-dash placeholder, which is what
/// a user with no rating in that category shows.
#[component]
pub fn EloCard(category: Category, value: Option<(u32, i32)>) -> impl IntoView {
    let rating = value.map(|(r, _)| r);
    let diff = value.map(|(_, d)| d);

    view! {
        <div class="flex flex-col gap-3 p-5 surface-card
                    hover:border-zinc-700 hover:-translate-y-px active:translate-y-0
                    transition-all duration-200 min-w-[128px] cursor-default
                    shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]">
            {category_icon(category)}
            <div class="flex flex-col gap-1">
                <span class="text-3xl font-bold tracking-tighter text-white leading-none">
                    {rating.map(|r| r.to_string()).unwrap_or_else(|| "\u{2014}".to_string())}
                </span>
                <span class=match diff {
                    Some(d) if d > 0 => "text-[11px] font-semibold text-emerald-400/80",
                    Some(d) if d < 0 => "text-[11px] font-semibold text-red-400/80",
                    _ => "text-[11px] font-semibold text-zinc-600",
                }>
                    {match diff {
                        Some(d) if d > 0 => format!("+{d}"),
                        Some(d) if d < 0 => format!("{d}"),
                        _ => "\u{2014}".to_string(),
                    }}
                </span>
                <span class="eyebrow-sm text-zinc-500">
                    {category.to_string().to_case(Case::Title)}
                </span>
            </div>
        </div>
    }
}
