use leptos::prelude::*;
use leptos_router::components::{Outlet, Redirect};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Client-facing snapshot of the signed-in user. Server-only fields
/// (password_hash, created_at) are intentionally omitted.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserSummary {
    pub id: Uuid,
    pub email: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub country: Option<String>,
}

#[server]
pub async fn current_user() -> Result<Option<UserSummary>, ServerFnError> {
    use crate::auth::AuthBackend;
    use axum_login::AuthSession;
    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    Ok(auth.user.map(|u| UserSummary {
        id: u.id,
        email: u.email,
        username: u.username,
        avatar_url: u.avatar_url,
        bio: u.bio,
        country: u.country,
    }))
}

#[server]
pub async fn logout() -> Result<(), ServerFnError> {
    use crate::auth::AuthBackend;
    use axum_login::AuthSession;
    let mut auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    auth.logout().await?;
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

    // `Transition` instead of `Suspense`: keep the previously rendered DOM
    // mounted while inner resources refetch. Otherwise any resource read
    // inside the Outlet (e.g. the username-availability check on the
    // create-username page) would unmount this whole subtree on every fetch
    // — losing focus on inputs, scroll position, etc.
    view! {
        <Transition>
            {move || user.get().map(|res| match res {
                Ok(Some(user)) => {
                    let on_username_page = location.pathname.get() == "/create-username";
                    match user.username {
                        // Onboarded — let the route render.
                        Some(_) => view! { <Outlet/> }.into_any(),
                        // Not onboarded, already on the username page — render it
                        // (otherwise we'd redirect to ourselves forever).
                        None if on_username_page => view! { <Outlet/> }.into_any(),
                        // Not onboarded, on any other route — force them through onboarding.
                        None => view! { <Redirect path="/create-username"/> }.into_any(),
                    }
                },
                _ => view! { <Redirect path="/login"/> }.into_any(),
            })}
        </Transition>
    }
}
