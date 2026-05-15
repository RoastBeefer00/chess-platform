use leptos::prelude::*;
use leptos_router::{lazy_route, LazyRoute};

#[server]
pub async fn login_with_password(email: String, password: String) -> Result<(), ServerFnError> {
    use crate::auth::{AuthBackend, Credentials};
    use axum_login::AuthSession;
    let mut auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    match auth
        .authenticate(Credentials::Password { email, password })
        .await?
    {
        Some(user) => {
            auth.login(&user).await?;
            leptos_axum::redirect("/");
            Ok(())
        }
        None => Err(ServerFnError::ServerError(
            "Invalid email or password".to_string(),
        )),
    }
}

#[derive(Clone)]
pub struct LoginPage;

#[lazy_route]
impl LazyRoute for LoginPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let action = ServerAction::<LoginWithPassword>::new();
        let value = action.value();
        let error = Memo::new(move |_| value.get().and_then(|r| r.err()).map(|e| e.to_string()));
        let pending = action.pending();

        // Full page navigation: `RequireAuth` uses `<Transition>` and would
        // serve the stale unauthenticated user during a client-side refetch,
        // bouncing us back to `/login` before the new session arrives.
        Effect::new(move |_| {
            if value.get().is_some_and(|r| r.is_ok()) {
                if let Some(win) = web_sys::window() {
                    let _ = win.location().set_href("/");
                }
            }
        });

        view! {
            <div class="flex flex-col items-center justify-center min-h-[calc(100vh-3.5rem)] px-6">
                <div class="w-full max-w-sm">
                    <h1 class="text-2xl font-semibold text-white mb-8 text-center">"Sign in"</h1>

                    <div class="space-y-3">
                        <a
                            href="/auth/google"
                            rel="external"
                            class="flex items-center justify-center gap-3 w-full py-2.5 px-4 bg-zinc-900 border border-zinc-700 text-white font-medium rounded-md hover:bg-zinc-800 transition-colors"
                        >
                            <svg class="w-5 h-5" viewBox="0 0 24 24" aria-hidden="true">
                                <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"/>
                                <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"/>
                                <path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"/>
                                <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"/>
                            </svg>
                            "Continue with Google"
                        </a>
                        <a
                            href="/auth/github"
                            rel="external"
                            class="flex items-center justify-center gap-3 w-full py-2.5 px-4 bg-zinc-900 border border-zinc-700 text-white font-medium rounded-md hover:bg-zinc-800 transition-colors"
                        >
                            <svg class="w-5 h-5 fill-current" viewBox="0 0 24 24" aria-hidden="true">
                                <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z"/>
                            </svg>
                            "Continue with GitHub"
                        </a>
                    </div>

                    <div class="flex items-center gap-3 my-6">
                        <div class="flex-1 h-px bg-zinc-800"></div>
                        <span class="text-zinc-500 text-sm">"or"</span>
                        <div class="flex-1 h-px bg-zinc-800"></div>
                    </div>

                    <ActionForm action=action>
                        <div class="space-y-4">
                            <div>
                                <label class="block text-sm text-zinc-400 mb-1" for="email">"Email"</label>
                                <input
                                    id="email"
                                    type="email"
                                    name="email"
                                    required
                                    class="w-full px-3 py-2 bg-zinc-900 border border-zinc-700 rounded-md text-white placeholder-zinc-500 focus:outline-none focus:border-zinc-500"
                                    placeholder="you@example.com"
                                />
                            </div>
                            <div>
                                <label class="block text-sm text-zinc-400 mb-1" for="password">"Password"</label>
                                <input
                                    id="password"
                                    type="password"
                                    name="password"
                                    required
                                    class="w-full px-3 py-2 bg-zinc-900 border border-zinc-700 rounded-md text-white placeholder-zinc-500 focus:outline-none focus:border-zinc-500"
                                    placeholder="••••••••"
                                />
                            </div>
                            <Show when=move || error.get().is_some()>
                                <p class="text-red-400 text-sm">{move || error.get()}</p>
                            </Show>
                            <button
                                type="submit"
                                disabled=pending
                                class="w-full py-2.5 px-4 bg-white text-zinc-950 font-medium rounded-md hover:bg-zinc-100 transition-colors disabled:opacity-50"
                            >
                                "Sign in"
                            </button>
                        </div>
                    </ActionForm>

                    <p class="text-center text-zinc-500 text-sm mt-6">
                        "Don't have an account? "
                        <a href="/register" class="text-white hover:underline">"Register"</a>
                    </p>
                </div>
            </div>
        }
        .into_any()
    }
}
