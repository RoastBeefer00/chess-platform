use leptos::prelude::*;

#[component]
pub fn Nav() -> impl IntoView {
    view! {
        <nav class="fixed top-0 left-0 w-full z-50 border-b border-zinc-800 bg-zinc-950/90 backdrop-blur-sm">
            <div class="flex items-center justify-between px-6 h-14 max-w-7xl mx-auto">
                <a href="/" class="flex items-center gap-2.5 shrink-0">
                    <img src="/assets/logo.svg" alt="Logo" width="32" height="32"/>
                    <span class="font-semibold text-white tracking-tight text-sm">"chess-rs"</span>
                </a>

                <div class="flex items-center gap-1 text-sm font-medium text-zinc-400">
                    <a href="/" class="px-3 py-1.5 rounded-md hover:text-white hover:bg-zinc-800 transition-colors">"Play"</a>
                    <a href="/puzzles" class="px-3 py-1.5 rounded-md hover:text-white hover:bg-zinc-800 transition-colors">"Puzzles"</a>
                    <a href="/learn" class="px-3 py-1.5 rounded-md hover:text-white hover:bg-zinc-800 transition-colors">"Learn"</a>
                    <a href="/watch" class="px-3 py-1.5 rounded-md hover:text-white hover:bg-zinc-800 transition-colors">"Watch"</a>
                </div>

                <div class="flex items-center gap-2 shrink-0">
                    <button class="px-3.5 py-1.5 text-sm font-medium text-zinc-300 rounded-md hover:text-white hover:bg-zinc-800 transition-colors">
                        "Log in"
                    </button>
                    <button class="px-3.5 py-1.5 text-sm font-medium text-zinc-950 bg-white rounded-md hover:bg-zinc-100 transition-colors">
                        "Register"
                    </button>
                </div>
            </div>
        </nav>
    }
}
