use leptos::prelude::*;
use leptos_router::components::{Outlet, Redirect};

#[server]
pub async fn current_user() -> Result<Option<String>, ServerFnError> {
    use crate::auth::AuthBackend;
    use axum_login::AuthSession;
    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    Ok(auth.user.map(|u| u.email))
}

#[server]
pub async fn logout() -> Result<(), ServerFnError> {
    use crate::auth::AuthBackend;
    use axum_login::AuthSession;
    let mut auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    auth.logout().await?;
    leptos_axum::redirect("/login");
    Ok(())
}

#[component]
pub fn RequireAuth() -> impl IntoView {
    let user = Resource::new(|| (), |_| current_user());

    view! {
        <Suspense>
            {move || user.get().map(|res| match res {
                Ok(Some(_)) => view! { <Outlet/> }.into_any(),
                _ => view! { <Redirect path="/login"/> }.into_any(),
            })}
        </Suspense>
    }
}
