use leptos::prelude::*;
use shared::{Category, RatingMode, TimeControl, TimeMode};
use strum::IntoEnumIterator;

use crate::components::{EloCard, MatchmakingModal};

#[component]
pub fn PlayHub() -> impl IntoView {
    let searching = RwSignal::new(None::<(TimeControl, RatingMode)>);

    let time_groups = [
        ("Bullet", vec![
            ("1+0", 60_000i64, 0i64),
            ("1+1", 60_000, 1_000),
            ("2+1", 120_000, 1_000),
        ]),
        ("Blitz", vec![
            ("3+0", 180_000, 0),
            ("3+2", 180_000, 2_000),
            ("5+0", 300_000, 0),
            ("5+5", 300_000, 5_000),
        ]),
        ("Rapid", vec![
            ("10+0", 600_000, 0),
            ("15+10", 900_000, 10_000),
        ]),
    ];

    view! {
        <div class="flex flex-col min-h-[100dvh]">
            // EloCards strip
            <div class="flex gap-3 overflow-x-auto scrollbar-none px-6 pt-6 pb-4">
                {Category::iter().map(|c| view! {
                    <EloCard category={c} />
                }).collect_view()}
            </div>

            // Time control section
            <div class="px-6 py-8">
                <h1 class="text-3xl font-bold tracking-tighter text-white mb-8">
                    "Quick match"
                </h1>

                <div class="flex flex-col gap-1">
                    {time_groups.into_iter().map(|(label, controls)| view! {
                        <div class="flex flex-col gap-3 py-4 border-t border-zinc-800/50">
                            <span class="text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-500">
                                {label}
                            </span>
                            <div class="flex flex-wrap gap-2">
                                {controls.into_iter().map(|(name, ms, inc)| view! {
                                    <button
                                        on:click=move |_| searching.set(Some((
                                            TimeControl { initial_time: ms, mode: TimeMode::Increment(inc) },
                                            RatingMode::Rated,
                                        )))
                                        class="px-4 py-3 rounded-xl bg-zinc-900 border border-zinc-800/60
                                               hover:bg-zinc-800 hover:border-zinc-700
                                               active:scale-[0.97] transition-all duration-150 cursor-pointer
                                               shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]"
                                    >
                                        <span class="text-base font-bold tracking-tighter text-white leading-none">
                                            {name}
                                        </span>
                                    </button>
                                }).collect_view()}
                            </div>
                        </div>
                    }).collect_view()}
                </div>
            </div>
        </div>

        <Show when=move || searching.get().is_some()>
            {move || searching.get().map(|(time_control, rating_mode)| view! {
                <MatchmakingModal
                    time_control={time_control}
                    rating_mode={rating_mode}
                    on_close=move |_| searching.set(None)
                />
            })}
        </Show>
    }
}
