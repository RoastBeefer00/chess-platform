use leptos::prelude::*;

use crate::components::auth::{use_current_user, Logout};

#[component]
pub fn UserMenu() -> impl IntoView {
    let user = use_current_user();
    let logout_action = ServerAction::<Logout>::new();
    let logout_value = logout_action.value();

    // Logout is session-changing — same race as login/register: a client-side
    // navigation would leave the stale `current_user` showing the signed-in
    // state. Full reload keeps the nav honest.
    Effect::new(move |_| {
        if logout_value.get().is_some_and(|r| r.is_ok()) {
            if let Some(win) = web_sys::window() {
                let _ = win.location().set_href("/login");
            }
        }
    });

    view! {
        <Suspense fallback=|| view! {
            <div class="w-24 h-8 bg-zinc-800 rounded-md animate-pulse"/>
        }>
            {move || user.get().map(|res| match res {
                Ok(Some(user)) => {
                    let display = user.username.clone().unwrap_or_else(|| {
                        user.email
                            .split('@')
                            .next()
                            .unwrap_or(&user.email)
                            .to_string()
                    });
                    let is_guest = user.is_guest;
                    view! {
                        <div class="flex items-center gap-2">
                            <Show when=move || is_guest>
                                <span class="px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-zinc-400 border border-zinc-700 rounded">
                                    "Guest"
                                </span>
                            </Show>
                            <span class="text-sm text-zinc-400">{display}</span>
                            <ActionForm action=logout_action>
                                <button
                                    type="submit"
                                    class="px-3.5 py-1.5 text-sm font-medium text-zinc-300 rounded-md hover:text-white hover:bg-zinc-800 transition-colors"
                                >
                                    {move || if is_guest { "Sign in" } else { "Sign out" }}
                                </button>
                            </ActionForm>
                        </div>
                    }.into_any()
                }
                _ => view! {
                    <a
                        href="/login"
                        class="px-3.5 py-1.5 text-sm font-medium text-zinc-950 bg-white rounded-md hover:bg-zinc-100 transition-colors"
                    >
                        "Log in"
                    </a>
                }.into_any()
            })}
        </Suspense>
    }
}
