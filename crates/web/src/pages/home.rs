use leptos::prelude::*;

#[component]
pub fn HomePage() -> impl IntoView {
    view! {
        <div class="flex flex-col items-center justify-center min-h-[calc(100vh-3.5rem)] px-6 text-center">
            <h1 class="text-5xl font-semibold tracking-tight text-white mb-3">
                "Your next move"
            </h1>
            <p class="text-zinc-400 text-lg mb-8 max-w-sm">
                "Play, learn, and improve — all in one place."
            </p>
            <div class="flex items-center gap-3">
                <a href="/play"
                   class="px-5 py-2.5 text-sm font-medium bg-white text-zinc-950 rounded-md hover:bg-zinc-100 transition-colors">
                    "Play now"
                </a>
                <a href="/learn"
                   class="px-5 py-2.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors">
                    "Learn"
                </a>
            </div>
        </div>
    }
}
