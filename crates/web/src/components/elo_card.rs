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
) -> Result<RatingsFor, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    // Signed-in callers only, in both branches. Ratings are shown on the
    // profile page, which lives behind `RequireAuth` — but that gate is in
    // the UI, and a server fn is an HTTP endpoint anyone can call. The
    // `Some(name)` branch used to skip this check entirely, so a signed-out
    // request could read any account's ratings.
    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let Some(viewer_id) = auth.user.as_ref().map(|u| u.id) else {
        return Err(ServerFnError::new("not signed in"));
    };
    let app_state = expect_context::<AppState>();
    let (id, name) = match username {
        Some(name) => {
            let u = app_state
                .user_store
                .find_by_username(&name)
                .await?
                .ok_or_else(|| ServerFnError::new("user not found"))?;
            (u.id, u.username)
        }
        None => (viewer_id, auth.user.as_ref().and_then(|u| u.username.clone())),
    };
    let ratings = app_state
        .rating_store
        .get_all_ratings_with_diff(&id)
        .await
        .map_err(|e| {
            tracing::error!(%id, error = %e, "get_all_ratings failed");
            ServerFnError::new(format!("Error getting ratings for user {id}: {e}"))
        })?;
    Ok(RatingsFor { username: name, ratings })
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

/// What `get_all_ratings` returns: the ratings, plus the username they belong
/// to so each card can link to that user's stats page.
///
/// The username rides along rather than being fetched separately because the
/// server fn has already resolved the user to read their ratings. Asking the
/// client to pair this with a second resource is exactly the nested-resource
/// shape that desynced SSR and hydration here before. `None` for an account
/// that has not finished onboarding, which has no profile URL to link to.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct RatingsFor {
    pub username: Option<String>,
    pub ratings: Ratings,
}

#[derive(Copy, Clone)]
pub struct MyRatingsResource(pub Resource<Result<RatingsFor, ServerFnError>>);

/// Call once at the App root.
pub fn provide_my_ratings() {
    provide_context(MyRatingsResource(Resource::new(
        || (),
        |_| async move { get_all_ratings(None).await },
    )));
}

pub fn use_my_ratings() -> Resource<Result<RatingsFor, ServerFnError>> {
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
                let found: RatingsFor = ratings.get().and_then(|r| r.ok()).unwrap_or_default();
                let owner = found.username.clone();
                Category::iter()
                    .map(|category| {
                        let value = found
                            .ratings
                            .iter()
                            .find(|(c, _, _)| *c == category)
                            .map(|(_, rating, diff)| (*rating, *diff));
                        // Only linkable once we know whose ratings these are;
                        // an account mid-onboarding has no profile URL.
                        let href = owner
                            .as_ref()
                            .map(|u| format!("/stats/{u}/{category}"));
                        view! { <EloCard category=category value=value href=href/> }
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
/// `href` of `None` renders the card as a plain `div`. The card already had
/// hover affordances — a border lift and a nudge upward — while being
/// `cursor-default` and inert, so it looked pressable without being
/// pressable. With a link it is both.
#[component]
pub fn EloCard(
    category: Category,
    value: Option<(u32, i32)>,
    // `optional_no_strip` rather than `optional`: the latter strips the
    // `Option` and makes callers pass a bare `String`, but the caller here
    // computes an `Option` (there is no link for an account without a
    // username) and wants to hand it straight through.
    #[prop(optional_no_strip)]
    href: Option<String>,
) -> impl IntoView {
    let rating = value.map(|(r, _)| r);
    let diff = value.map(|(_, d)| d);
    let shell = "flex flex-col gap-3 p-5 surface-card \
                 hover:border-zinc-700 hover:-translate-y-px active:translate-y-0 \
                 transition-all duration-200 min-w-[128px] \
                 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]";
    let body = view! {
        <>
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
        </>
    };

    match href {
        Some(href) => view! {
            <a href={href} class={format!("{shell} cursor-pointer")}>{body}</a>
        }
        .into_any(),
        None => view! { <div class={format!("{shell} cursor-default")}>{body}</div> }.into_any(),
    }
}
