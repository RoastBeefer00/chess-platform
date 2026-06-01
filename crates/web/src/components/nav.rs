use leptos::prelude::*;

use crate::components::user::UserMenu;

#[component]
pub fn Nav() -> impl IntoView {
    let mobile_open = RwSignal::new(false);
    let close_mobile = move |_| mobile_open.set(false);

    view! {
        <nav class="fixed top-0 left-0 w-full z-50 border-b border-zinc-800 bg-zinc-950/90 backdrop-blur-sm">
            <div class="flex items-center justify-between px-4 sm:px-6 h-14 max-w-7xl mx-auto">
                <a
                    href="/"
                    on:click=close_mobile
                    class="shrink-0 text-xl font-bold tracking-tight text-white hover:text-zinc-200 transition-colors"
                >
                    "gambit.rs"
                </a>

                // Desktop links — hidden on mobile.
                <div class="hidden md:flex items-center gap-1 text-sm font-medium text-zinc-400">
                    <a href="/" class="px-3 py-1.5 rounded-md hover:text-white hover:bg-zinc-800 transition-colors">"Play"</a>
                    <a href="/puzzles" class="px-3 py-1.5 rounded-md hover:text-white hover:bg-zinc-800 transition-colors">"Puzzles"</a>
                    <a href="/learn" class="px-3 py-1.5 rounded-md hover:text-white hover:bg-zinc-800 transition-colors">"Learn"</a>
                    <a href="/watch" class="px-3 py-1.5 rounded-md hover:text-white hover:bg-zinc-800 transition-colors">"Watch"</a>
                </div>

                <div class="flex items-center gap-2 shrink-0">
                    <UserMenu/>
                    // Hamburger — only shown on mobile.
                    <button
                        on:click=move |_| mobile_open.update(|v| *v = !*v)
                        class="md:hidden w-9 h-9 flex items-center justify-center rounded-md text-zinc-400 hover:text-white hover:bg-zinc-800 transition-colors"
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
                        <a href="/" on:click=close_mobile class="px-3 py-3 text-sm font-medium text-zinc-300 hover:text-white hover:bg-zinc-800 rounded-md transition-colors">"Play"</a>
                        <a href="/puzzles" on:click=close_mobile class="px-3 py-3 text-sm font-medium text-zinc-300 hover:text-white hover:bg-zinc-800 rounded-md transition-colors">"Puzzles"</a>
                        <a href="/learn" on:click=close_mobile class="px-3 py-3 text-sm font-medium text-zinc-300 hover:text-white hover:bg-zinc-800 rounded-md transition-colors">"Learn"</a>
                        <a href="/watch" on:click=close_mobile class="px-3 py-3 text-sm font-medium text-zinc-300 hover:text-white hover:bg-zinc-800 rounded-md transition-colors">"Watch"</a>
                    </div>
                </div>
            </Show>
        </nav>
    }
}
