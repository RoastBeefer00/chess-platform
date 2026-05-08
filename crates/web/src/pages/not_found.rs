use leptos::prelude::*;

#[component]
pub fn NotFoundPage() -> impl IntoView {
    view! {
        <div class="flex flex-col items-center justify-center min-h-[calc(100vh-3.5rem)] px-6 text-center select-none">
            <span class="text-[10rem] font-bold leading-none tracking-tighter text-zinc-800 mb-2">
                "404"
            </span>
            <h1 class="text-2xl font-semibold tracking-tight text-white mb-2">
                "Empty square"
            </h1>
            <p class="text-zinc-500 text-sm mb-8 max-w-xs">
                "This square is unoccupied. Make your next move from somewhere that exists."
            </p>
            <a href="/"
               class="px-5 py-2.5 text-sm font-medium bg-white text-zinc-950 rounded-md hover:bg-zinc-100 transition-colors">
                "Back to board"
            </a>
        </div>
    }
}
