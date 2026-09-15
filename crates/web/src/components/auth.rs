use leptos::prelude::*;
use leptos_router::components::{Outlet, Redirect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Client-facing snapshot of the signed-in user. Server-only fields
/// (password_hash, created_at) are intentionally omitted.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UserSummary {
    pub id: Uuid,
    pub email: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub country: Option<String>,
    pub is_guest: bool,
    pub settings: shared::UserSettings,
}

#[server]
pub async fn current_user() -> Result<Option<UserSummary>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;
    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let Some(u) = auth.user else { return Ok(None) };
    let state = expect_context::<AppState>();
    let settings = state.user_store.get_settings(u.id).await?;
    Ok(Some(UserSummary {
        id: u.id,
        email: u.email,
        username: u.username,
        avatar_url: u.avatar_url,
        bio: u.bio,
        country: u.country,
        is_guest: u.is_guest,
        settings,
    }))
}

#[server]
pub async fn logout() -> Result<(), ServerFnError> {
    use crate::auth::AuthBackend;
    use axum_login::AuthSession;
    let mut auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let user_id = auth.user.as_ref().map(|u| u.id);
    auth.logout().await?;
    if let Some(id) = user_id {
        tracing::info!(user_id = %id, "logout");
    }
    // No `leptos_axum::redirect` here on purpose: the client-side `UserMenu`
    // does a full `window.location` reload after this action succeeds, so a
    // server redirect would just trigger a redundant client-side navigation
    // that briefly shows the stale signed-in nav.
    Ok(())
}

/// Newtype wrapper so the resource can be looked up by a distinct type
/// in `use_context`. The inner `Resource` is `Copy`, so this is cheap to clone.
#[derive(Copy, Clone)]
pub struct CurrentUserResource(pub Resource<Result<Option<UserSummary>, ServerFnError>>);

/// Bump this signal to force the current-user resource to refetch
/// (e.g. after a successful client-side login/logout).
#[derive(Copy, Clone)]
pub struct AuthTrigger(pub RwSignal<u64>);

/// Call once at the App root.
pub fn provide_current_user() {
    let trigger = RwSignal::new(0u64);
    let user = Resource::new(move || trigger.get(), |_| current_user());
    provide_context(CurrentUserResource(user));
    provide_context(AuthTrigger(trigger));
}

pub fn use_current_user() -> Resource<Result<Option<UserSummary>, ServerFnError>> {
    use_context::<CurrentUserResource>()
        .expect("provide_current_user must be called at the App root")
        .0
}

pub fn use_auth_trigger() -> AuthTrigger {
    use_context::<AuthTrigger>().expect("provide_current_user must be called at the App root")
}

#[component]
pub fn RequireAuth() -> impl IntoView {
    let user = use_current_user();
    let location = leptos_router::hooks::use_location();

    // `<Show>`'s `when` is read directly off the resource *inside*
    // `<Transition>`'s children (required — reading a resource via a `Memo`
    // built outside a Suspense/Transition boundary, even one whose value
    // feeds a view rendered inside one, logs a "reading a resource outside
    // Suspense/Transition" warning and forgoes SSR's wait-for-resolution).
    // `<Show>` has its own internal dedup on `when`'s boolean output, so
    // repeated resolutions that don't change the boolean (a settings save
    // bumping `AuthTrigger`, refetching `current_user`, while still signed
    // in as the same user) don't reconstruct `<Outlet/>` — which tears down
    // and re-fetches every resource on the entire routed page from scratch
    // (this is what made the profile page intermittently go blank/empty
    // right after toggling a setting). The other branches (redirects, the
    // create-username form) have no comparable state worth protecting, so
    // they're left as a plain match in the `fallback`.
    let onboarded = move || matches!(&user.get(), Some(Ok(Some(u))) if u.username.is_some());

    // `Transition` instead of `Suspense`: keep the previously rendered DOM
    // mounted while inner resources refetch. Otherwise any resource read
    // inside the Outlet (e.g. the username-availability check on the
    // create-username page) would unmount this whole subtree on every fetch
    // — losing focus on inputs, scroll position, etc.
    view! {
        <Transition>
            <Show
                when=onboarded
                fallback=move || match user.get() {
                    // Still genuinely pending (first load) — render nothing
                    // rather than redirecting; `<Transition>` covers this in
                    // practice since SSR always waits for resolution.
                    None => ().into_any(),
                    // Signed in, but hasn't picked a username yet.
                    Some(Ok(Some(_))) => if location.pathname.get() == "/create-username" {
                        // Already on the username page — render it
                        // (otherwise we'd redirect to ourselves forever).
                        view! { <Outlet/> }.into_any()
                    } else {
                        // Force them through onboarding.
                        view! { <Redirect path="/create-username"/> }.into_any()
                    },
                    _ => view! { <Redirect path="/login"/> }.into_any(),
                }
            >
                <Outlet/>
            </Show>
        </Transition>
    }
}
