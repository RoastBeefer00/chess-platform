use leptos::prelude::*;
use leptos_router::{lazy_route, LazyRoute};
use shared::{RatingMode, TimeControl, TimeMode};

use crate::matchmaking::use_start_matchmaking;

#[derive(Clone)]
pub struct HomePage;

#[lazy_route]
impl LazyRoute for HomePage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let start_matchmaking = use_start_matchmaking();

        let time_controls = vec![
            ("1+0", "Bullet", 60_000, 0),
            ("1+1", "Bullet", 60_000, 1_000),
            ("2+1", "Bullet", 120_000, 1_000),
            ("3+0", "Blitz", 180_000, 0),
            ("3+2", "Blitz", 180_000, 2_000),
            ("5+0", "Blitz", 300_000, 0),
            ("5+5", "Blitz", 300_000, 5_000),
            ("10+0", "Rapid", 600_000, 0),
            ("15+10", "Rapid", 900_000, 10_000),
        ];
        view! {
            <div class="flex flex-col items-center justify-center min-h-[calc(100dvh-3.5rem)] px-6 text-center">
                <h1 class="text-5xl font-semibold tracking-tight text-white mb-3">
                    "Your next move"
                </h1>
                <p class="text-zinc-400 text-lg mb-8 max-w-sm">
                    "Play, learn, and improve — all in one place."
                </p>
                <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-5 gap-3 w-full max-w-2xl">
                    {time_controls.into_iter().map(|(name, category, ms, inc)| view! {
                        <button
                            on:click=move |_| start_matchmaking.run((
                                TimeControl { initial_time: ms, mode: TimeMode::Increment(inc) },
                                RatingMode::Rated,
                            ))
                            class="group flex flex-col items-center justify-center gap-1 px-4 py-5 rounded-lg bg-zinc-900 border border-zinc-800 hover:border-zinc-600 hover:bg-zinc-800 transition-colors"
                        >
                            <span class="text-2xl font-bold tracking-tight text-white">{name}</span>
                            <span class="text-xs font-medium uppercase tracking-wider text-zinc-500 group-hover:text-zinc-300 transition-colors">{category}</span>
                        </button>
                    }).collect_view()}
                </div>
            </div>
        }
        .into_any()
    }
}
