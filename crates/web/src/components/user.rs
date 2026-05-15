use leptos::prelude::*;

use crate::components::auth::{current_user, Logout};

#[component]
pub fn UserMenu() -> impl IntoView {
    let user = Resource::new(|| (), |_| current_user());
    let logout_action = ServerAction::<Logout>::new();

    view! {
        <Suspense fallback=|| view! {
            <div class="w-24 h-8 bg-zinc-800 rounded-md animate-pulse"/>
        }>
            {move || user.get().map(|res| match res {
                Ok(Some(email)) => {
                    let display = email
                        .split('@')
                        .next()
                        .unwrap_or(&email)
                        .to_string();
                    view! {
                        <div class="flex items-center gap-2">
                            <span class="text-sm text-zinc-400">{display}</span>
                            <ActionForm action=logout_action>
                                <button
                                    type="submit"
                                    class="px-3.5 py-1.5 text-sm font-medium text-zinc-300 rounded-md hover:text-white hover:bg-zinc-800 transition-colors"
                                >
                                    "Sign out"
                                </button>
                            </ActionForm>
                        </div>
                    }.into_any()
                }
                _ => view! {
                    <div class="flex items-center gap-2">
                        <a
                            href="/login"
                            class="px-3.5 py-1.5 text-sm font-medium text-zinc-300 rounded-md hover:text-white hover:bg-zinc-800 transition-colors"
                        >
                            "Log in"
                        </a>
                        <a
                            href="/register"
                            class="px-3.5 py-1.5 text-sm font-medium text-zinc-950 bg-white rounded-md hover:bg-zinc-100 transition-colors"
                        >
                            "Register"
                        </a>
                    </div>
                }.into_any()
            })}
        </Suspense>
    }
}
