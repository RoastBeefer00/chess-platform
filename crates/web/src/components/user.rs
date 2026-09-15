use leptos::prelude::*;

use crate::components::auth::{use_current_user, Logout};

/// The "Profile"/"Settings" dropdown panel, gated by `<Show>` in `UserMenu`
/// so it's only ever constructed once `open` is `true` — same reason
/// `PromotionPickerCard` is split out and `Show`-gated: registering
/// `on_click_outside` against a `NodeRef` that might never attach (because
/// the element it's meant to attach to doesn't exist yet) crashes on mount.
#[component]
fn UserDropdown(username: String, open: RwSignal<bool>) -> impl IntoView {
    let menu_ref = NodeRef::<leptos::html::Div>::new();

    #[cfg(feature = "hydrate")]
    {
        let stop = leptos_use::on_click_outside(menu_ref, move |_| open.set(false));
        on_cleanup(stop);
    }

    let profile_href = format!("/u/{username}");

    view! {
        <div
            node_ref=menu_ref
            class="absolute right-0 top-full mt-2 w-40 flex flex-col rounded-md overflow-hidden shadow-xl bg-zinc-900 border border-zinc-800 z-50"
        >
            <a
                href={profile_href}
                on:click=move |_| open.set(false)
                class="px-4 py-2.5 text-sm text-zinc-300 hover:bg-zinc-800 hover:text-white transition-colors"
            >
                "Profile"
            </a>
            <a
                href="/settings"
                on:click=move |_| open.set(false)
                class="px-4 py-2.5 text-sm text-zinc-300 hover:bg-zinc-800 hover:text-white transition-colors"
            >
                "Settings"
            </a>
        </div>
    }
}

#[component]
pub fn UserMenu() -> impl IntoView {
    let user = use_current_user();
    let logout_action = ServerAction::<Logout>::new();
    let logout_value = logout_action.value();
    let dropdown_open = RwSignal::new(false);

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
        // `Transition`, not `Suspense`: keeps the previously rendered nav
        // (username, sign-out button) visible while `current_user` refetches
        // — e.g. every time the settings page bumps `AuthTrigger` on save.
        // `Suspense` would swap in its pulsing-skeleton fallback on every
        // such refetch, flickering the whole nav bar.
        <Transition fallback=|| view! {
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
                    let username = user.username.clone();
                    view! {
                        <div class="flex items-center gap-2">
                            <Show when=move || is_guest>
                                <span class="px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-zinc-400 border border-zinc-700 rounded">
                                    "Guest"
                                </span>
                            </Show>
                            {match username.clone() {
                                Some(username) => view! {
                                    <div class="relative">
                                        <button
                                            type="button"
                                            on:click=move |_| dropdown_open.update(|v| *v = !*v)
                                            class="text-sm text-zinc-400 hover:text-white transition-colors cursor-pointer"
                                        >
                                            {display}
                                        </button>
                                        <Show when=move || dropdown_open.get()>
                                            <UserDropdown username=username.clone() open=dropdown_open/>
                                        </Show>
                                    </div>
                                }.into_any(),
                                None => view! { <span class="text-sm text-zinc-400">{display}</span> }.into_any(),
                            }}
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
        </Transition>
    }
}
