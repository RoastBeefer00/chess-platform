use leptos::prelude::*;
use leptos_router::{lazy_route, LazyRoute};

#[server]
pub async fn register_user(email: String, password: String) -> Result<(), ServerFnError> {
    use crate::auth::AuthBackend;
    use axum_login::AuthSession;

    let backend = expect_context::<AuthBackend>();
    let mut auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;

    let user = backend.register(email, password).await?;
    auth.login(&user).await?;

    leptos_axum::redirect("/");
    Ok(())
}

#[derive(Clone)]
pub struct RegisterPage;

#[lazy_route]
impl LazyRoute for RegisterPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let action = ServerAction::<RegisterUser>::new();
        let value = action.value();
        let error = Memo::new(move |_| {
            value.get().and_then(|r| r.err()).map(|e| e.to_string())
        });
        let pending = action.pending();

        // Full page navigation — see comment in login.rs.
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
                    <h1 class="text-2xl font-semibold text-white mb-8 text-center">"Create account"</h1>

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
                                "Create account"
                            </button>
                        </div>
                    </ActionForm>

                    <p class="text-center text-zinc-500 text-sm mt-6">
                        "Already have an account? "
                        <a href="/login" class="text-white hover:underline">"Sign in"</a>
                    </p>
                </div>
            </div>
        }
        .into_any()
    }
}
