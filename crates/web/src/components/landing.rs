use leptos::prelude::*;

#[component]
pub fn Landing() -> impl IntoView {
    view! {
        <section class="relative min-h-[calc(100dvh-3.5rem)] flex items-center justify-center px-6 overflow-hidden">
            // Subtle radial spotlight behind the hero.
            <div class="absolute inset-0 pointer-events-none" style="background: radial-gradient(circle at center, rgba(255,255,255,0.06) 0%, transparent 60%);"></div>

            <div class="relative z-10 flex flex-col items-center text-center max-w-2xl">
                <h1 class="text-6xl sm:text-7xl font-semibold tracking-tighter text-white mb-4">
                    "Pure chess."
                </h1>
                <p class="text-zinc-400 text-lg sm:text-xl mb-10 max-w-lg">
                    "Free, fast, and open. No ads, no tracking, no nonsense."
                </p>
                <div class="flex flex-col sm:flex-row items-center gap-3">
                    <a
                        href="/register"
                        class="w-full sm:w-auto px-6 py-3 text-sm font-semibold bg-white text-zinc-950 rounded-md hover:bg-zinc-100 transition-colors"
                    >
                        "Get started"
                    </a>
                    <a
                        href="/login"
                        class="w-full sm:w-auto px-6 py-3 text-sm font-medium text-zinc-300 hover:text-white transition-colors"
                    >
                        "Sign in →"
                    </a>
                </div>
            </div>
        </section>
    }
}
