use leptos::prelude::*;

use crate::components::auth::use_current_user;
use crate::components::user::UserMenu;
use crate::friends::use_friends_presence;

/// Unanswered-friend-request count, as a small badge. Renders nothing at
/// zero so the nav is unchanged for everyone without pending requests.
#[component]
fn PendingBadge() -> impl IntoView {
    let presence = use_friends_presence();
    let count = move || presence.pending_requests.get();
    // Named rather than inlined into `when=`: an unparenthesized `>` there
    // is ambiguous with the RSX tag-close `>`, same trap as `WatchGrid`'s
    // `has_more_than_shown`.
    let has_pending = move || count() > 0;
    view! {
        <Show when=has_pending>
            <span
                class="ml-1.5 inline-flex items-center justify-center min-w-4 h-4 px-1
                       rounded-full bg-emerald-500 text-zinc-950 text-[10px] font-bold leading-none"
                aria-label=move || format!("{} pending friend requests", count())
            >
                {move || if count() > 9 { "9+".to_string() } else { count().to_string() }}
            </span>
        </Show>
    }
}

#[component]
pub fn Nav() -> impl IntoView {
    let mobile_open = RwSignal::new(false);
    let close_mobile = move |_| mobile_open.set(false);
    // Friends is sign-in-only (guests are excluded from the whole feature),
    // so the link is hidden rather than shown-and-erroring for everyone else.
    //
    // Read inside the `<Transition>`s below, not here: reading a resource
    // outside a Suspense boundary is a hydration-mismatch hazard, and Leptos
    // warns about it at runtime. `fallback=|| ()` renders nothing while it
    // resolves, which is also what we want — the link should appear once we
    // know it applies, not flicker in and out.
    let user = use_current_user();
    let show_friends = move || {
        user.get()
            .and_then(|r| r.ok())
            .flatten()
            .is_some_and(|u| !u.is_guest && u.username.is_some())
    };

    view! {
        <nav class="fixed top-0 left-0 w-full z-50 border-b border-zinc-800 bg-zinc-950/90 backdrop-blur-sm">
            // `h-nav` rather than a literal height: `--nav-h` in main.css is
            // the single source every "fill the viewport below the nav"
            // calculation reads from.
            <div class="flex items-center justify-between px-4 sm:px-6 h-nav max-w-7xl mx-auto">
                <a
                    href="/"
                    on:click=close_mobile
                    class="shrink-0 text-xl font-bold tracking-tight text-white hover:text-zinc-200 transition-colors"
                >
                    "gambit.rs"
                </a>

                // Desktop links — hidden on mobile.
                <div class="hidden md:flex items-center gap-1 text-sm font-medium text-zinc-400">
                    <a href="/" class="px-3 py-1.5 rounded-control hover:text-white hover:bg-zinc-800 transition-colors">"Play"</a>
                    <a href="/analysis" class="px-3 py-1.5 rounded-control hover:text-white hover:bg-zinc-800 transition-colors">"Analysis"</a>
                    <a href="/puzzles" class="px-3 py-1.5 rounded-control hover:text-white hover:bg-zinc-800 transition-colors">"Puzzles"</a>
                    <a href="/watch" class="px-3 py-1.5 rounded-control hover:text-white hover:bg-zinc-800 transition-colors">"Watch"</a>
                    <Transition fallback=|| ()>
                        <Show when=show_friends>
                            <a href="/friends" class="flex items-center px-3 py-1.5 rounded-control hover:text-white hover:bg-zinc-800 transition-colors">
                                "Friends"
                                <PendingBadge/>
                            </a>
                        </Show>
                    </Transition>
                </div>

                <div class="flex items-center gap-2 shrink-0">
                    <UserMenu/>
                    // Hamburger — only shown on mobile.
                    <button
                        on:click=move |_| mobile_open.update(|v| *v = !*v)
                        class="md:hidden w-9 h-9 flex items-center justify-center rounded-control text-zinc-400 hover:text-white hover:bg-zinc-800 transition-colors"
                        aria-label="Toggle menu"
                    >
                        {move || if mobile_open.get() { "✕" } else { "☰" }}
                    </button>
                </div>
            </div>

            // Mobile dropdown — collapses below the nav bar.
            <Show when=move || mobile_open.get()>
                <div class="md:hidden border-t border-zinc-800 bg-zinc-950">
                    <div class="flex flex-col px-4 py-2 max-w-7xl mx-auto">
                        <a href="/" on:click=close_mobile class="px-3 py-3 text-sm font-medium text-zinc-300 hover:text-white hover:bg-zinc-800 rounded-control transition-colors">"Play"</a>
                        <a href="/analysis" on:click=close_mobile class="px-3 py-3 text-sm font-medium text-zinc-300 hover:text-white hover:bg-zinc-800 rounded-control transition-colors">"Analysis"</a>
                        <a href="/puzzles" on:click=close_mobile class="px-3 py-3 text-sm font-medium text-zinc-300 hover:text-white hover:bg-zinc-800 rounded-control transition-colors">"Puzzles"</a>
                        <a href="/watch" on:click=close_mobile class="px-3 py-3 text-sm font-medium text-zinc-300 hover:text-white hover:bg-zinc-800 rounded-control transition-colors">"Watch"</a>
                        <Transition fallback=|| ()>
                            <Show when=show_friends>
                                <a href="/friends" on:click=close_mobile class="flex items-center px-3 py-3 text-sm font-medium text-zinc-300 hover:text-white hover:bg-zinc-800 rounded-control transition-colors">
                                    "Friends"
                                    <PendingBadge/>
                                </a>
                            </Show>
                        </Transition>
                    </div>
                </div>
            </Show>
        </nav>
    }
}
