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
    use axum_login::AuthSession;
    // `settings` rides along on the user row the session layer has already
    // loaded, so this server fn issues no query of its own.
    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let Some(u) = auth.user else { return Ok(None) };
    Ok(Some(UserSummary {
        id: u.id,
        email: u.email,
        username: u.username,
        avatar_url: u.avatar_url,
        bio: u.bio,
        country: u.country,
        is_guest: u.is_guest,
        settings: u.settings.0,
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

    // `<Outlet/>` is a *sibling* of the gate below, not its child.
    //
    // It used to be `<Show>`'s children inside this `<Transition>`, which
    // looked right — the protected page simply isn't built until the user is
    // known. But everything inside a `<Transition>` is rebuilt once per pass
    // of out-of-order streaming, three passes per SSR render, and rebuilding
    // the outlet re-creates every `Resource` on the routed page. Each of the
    // profile page's queries therefore ran three times for one page view
    // (25 queries per load; 9 once the outlet stopped being rebuilt).
    //
    // What made that safe to change is that this gate was never the actual
    // authorization boundary — every server fn behind it does its own check,
    // and the two that didn't (`get_all_ratings` and `get_recent_games`,
    // given an explicit username) were fixed alongside this. The gate's job
    // is to redirect, not to withhold data, so rendering the page in the
    // moments before a redirect lands leaks nothing: the server fns refuse,
    // and `<Redirect>` sets the response's `Location` before the body is
    // ever shown.
    //
    // `Transition` rather than `Suspense` so the redirect branch doesn't
    // unmount and remount as inner resources refetch — a resource read
    // inside the outlet (the username-availability check on the
    // create-username page) would otherwise blow this subtree away on every
    // fetch, losing input focus and scroll position.
    view! {
        <Transition>
            {move || match user.get() {
                // Still pending on first load: no verdict yet, so no
                // redirect. SSR waits for resolution before rendering.
                None => ().into_any(),
                // Signed in and onboarded — the outlet below is all they need.
                Some(Ok(Some(u))) if u.username.is_some() => ().into_any(),
                // Signed in, no username yet. Already on the onboarding page
                // means rendering it, not redirecting to ourselves forever.
                Some(Ok(Some(_))) if location.pathname.get() == "/create-username" => {
                    ().into_any()
                }
                Some(Ok(Some(_))) => view! { <Redirect path="/create-username"/> }.into_any(),
                _ => view! { <Redirect path="/login"/> }.into_any(),
            }}
        </Transition>
        <Outlet/>
    }
}
