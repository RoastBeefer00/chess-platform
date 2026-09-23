use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::{lazy_route, LazyRoute};

#[server]
pub async fn is_username_available(username: String) -> Result<bool, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    if auth.user.is_none() {
        return Err(ServerFnError::ServerError("unauthorized".to_string()));
    }
    // Reject junk before it reaches the DB — this doubles as the enumeration
    // guard, since a malformed name can never match a real one anyway.
    if shared::validate_username(&username).is_err() {
        return Ok(false);
    }
    let state = expect_context::<AppState>();
    Ok(state.user_store.is_username_available(username).await?)
}

#[server]
pub async fn set_username(username: String) -> Result<(), ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let mut auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let Some(user) = auth.user.clone() else {
        return Err(ServerFnError::ServerError("not signed in".to_string()));
    };
    let user_id = user.id;

    // A server function is a plain HTTP endpoint: the form's `pattern`/
    // `minlength` attributes and the live availability check are both
    // conveniences a hand-crafted POST skips straight past. This is the
    // only check that actually holds.
    if let Err(msg) = shared::validate_username(&username) {
        return Err(ServerFnError::ServerError(msg.to_string()));
    }

    // Onboarding is a one-shot claim, not a rename. Guests already have a
    // server-assigned `Guest<id>` name, so this is also what stops a
    // throwaway account from renaming itself into someone else's identity.
    if user.username.is_some() {
        return Err(ServerFnError::ServerError(
            "This account already has a username".to_string(),
        ));
    }

    let state = expect_context::<AppState>();
    state
        .user_store
        .set_username(user_id, username.clone())
        .await?;
    tracing::info!(user_id = %user_id, username = %username, "username_set");
    // Refresh the in-session user so AuthSession.user.username is up to date.
    if let Some(u) = state.user_store.find_by_id(&user_id).await? {
        auth.user = Some(u);
    }
    Ok(())
}

#[derive(Clone)]
pub struct CreateUsernamePage;

#[lazy_route]
impl LazyRoute for CreateUsernamePage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let action = ServerAction::<SetUsername>::new();
        let value = action.value();
        let error = Memo::new(move |_| value.get().and_then(|r| r.err()).map(|e| e.to_string()));
        let pending = action.pending();

        // Full page navigation rather than client-side `use_navigate` +
        // resource refetch: during the refetch, `RequireAuth` would observe
        // the stale user (still `username = None`) and bounce us back to
        // `/create-username` before the new value arrives.
        Effect::new(move |_| {
            if value.get().is_some_and(|r| r.is_ok()) {
                if let Some(win) = web_sys::window() {
                    let _ = win.location().set_href("/");
                }
            }
        });

        let (uname, set_uname) = signal(String::new());

        // Fires on every keystroke. Returns Ok(None) when the value can't
        // meaningfully be checked (too short, bad chars), so we don't hit
        // the server with junk.
        let availability = Resource::new(
            move || uname.get(),
            |u| async move {
                if shared::validate_username(&u).is_err() {
                    return Ok::<_, ServerFnError>(None);
                }
                is_username_available(u).await.map(Some)
            },
        );

        // Only enable submit once the server has confirmed the name is free.
        let submit_disabled =
            move || pending.get() || !matches!(availability.get(), Some(Ok(Some(true))));

        view! {
            <Title text="Pick a username"/>
            <div class="flex flex-col items-center justify-center min-h-below-nav px-6">
                <div class="w-full max-w-sm">
                    <h1 class="display-1 text-white mb-2 text-center">"Pick a username"</h1>
                    <p class="text-sm text-zinc-400 mb-8 text-center">
                        "This is how other players will see you."
                    </p>

                    <ActionForm action=action>
                        <div class="space-y-4">
                            <div>
                                <label class="block text-sm text-zinc-400 mb-1" for="username">"Username"</label>
                                <input
                                    id="username"
                                    type="text"
                                    name="username"
                                    required
                                    minlength="5"
                                    maxlength="32"
                                    pattern="[a-zA-Z0-9_]+"
                                    autocomplete="off"
                                    on:input=move |ev| set_uname.set(event_target_value(&ev))
                                    class="w-full px-3 py-2 bg-zinc-900 border border-zinc-700 rounded-control text-white placeholder-zinc-500 focus:outline-none focus:border-zinc-500"
                                    placeholder="Enter your username..."
                                />
                                {move || {
                                    let u = uname.get();
                                    if u.is_empty() {
                                        view! { <span/> }.into_any()
                                    } else if let Err(msg) = shared::validate_username(&u) {
                                        view! { <p class="text-amber-400 text-sm mt-1">{msg}</p> }.into_any()
                                    } else {
                                        match availability.get() {
                                            None | Some(Ok(None)) =>
                                                view! { <p class="text-zinc-500 text-sm mt-1">"Checking..."</p> }.into_any(),
                                            Some(Ok(Some(true))) =>
                                                view! { <p class="text-emerald-400 text-sm mt-1">"✓ Available"</p> }.into_any(),
                                            Some(Ok(Some(false))) =>
                                                view! { <p class="text-red-400 text-sm mt-1">"✗ Username taken"</p> }.into_any(),
                                            Some(Err(e)) =>
                                                view! { <p class="text-red-400 text-sm mt-1">{e.to_string()}</p> }.into_any(),
                                        }
                                    }
                                }}
                            </div>
                            <Show when=move || error.get().is_some()>
                                <p class="text-red-400 text-sm">{move || error.get()}</p>
                            </Show>
                            <button
                                type="submit"
                                disabled=submit_disabled
                                class="btn-primary w-full py-2.5 px-4 font-medium disabled:opacity-50 cursor-pointer"
                            >
                                "Continue"
                            </button>
                        </div>
                    </ActionForm>
                </div>
            </div>
        }
        .into_any()
    }
}
