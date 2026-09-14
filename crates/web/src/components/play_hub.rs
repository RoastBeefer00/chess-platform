use leptos::prelude::*;
use shared::{Category, RatingMode, TimeControl, TimeMode};
use strum::IntoEnumIterator;

use crate::components::{use_current_user, EloCard, MatchmakingModal, RecentGames};

const RATING_MODE_STORAGE_KEY: &str = "gambit:rating_mode";

#[component]
pub fn PlayHub() -> impl IntoView {
    let searching = RwSignal::new(None::<(TimeControl, RatingMode)>);
    let rating_mode = RwSignal::new(RatingMode::Rated);
    let user = use_current_user();
    let is_guest = move || {
        user.get()
            .and_then(|r| r.ok())
            .flatten()
            .is_some_and(|u| u.is_guest)
    };

    // Restore the last choice from localStorage. Runs client-side only
    // (post-hydration), never during render — the server has no `window`,
    // so a render-time read would make the server emit "Rated" and the
    // client immediately swap to "Casual", causing a hydration mismatch.
    Effect::new(move |_| {
        if is_guest() {
            rating_mode.set(RatingMode::Casual);
            return;
        }
        if let Some(win) = web_sys::window() {
            if let Ok(Some(store)) = win.local_storage() {
                if let Ok(Some(saved)) = store.get_item(RATING_MODE_STORAGE_KEY) {
                    if saved == "casual" {
                        rating_mode.set(RatingMode::Casual);
                    }
                }
            }
        }
    });

    let set_rating_mode = move |mode: RatingMode| {
        if is_guest() {
            return;
        }
        rating_mode.set(mode);
        if let Some(win) = web_sys::window() {
            if let Ok(Some(store)) = win.local_storage() {
                let _ = store.set_item(RATING_MODE_STORAGE_KEY, &mode.to_string());
            }
        }
    };

    let time_groups = [
        (
            "Bullet",
            vec![
                ("1 + 0", 60_000i64, 0i64),
                ("1 + 1", 60_000, 1_000),
                ("2 + 1", 120_000, 1_000),
            ],
        ),
        (
            "Blitz",
            vec![
                ("3 + 0", 180_000, 0),
                ("3 + 2", 180_000, 2_000),
                ("5 + 0", 300_000, 0),
                ("5 + 5", 300_000, 5_000),
            ],
        ),
        (
            "Rapid",
            vec![("10 + 0", 600_000, 0), ("15 + 10", 900_000, 10_000)],
        ),
    ];

    view! {
        <div class="flex flex-col min-h-[100dvh] max-w-2xl mx-auto">
            // EloCards strip
            <div class="flex gap-3 overflow-x-auto scrollbar-none px-6 pt-6 pb-4">
                {Category::iter().map(|c| view! {
                    <EloCard category={c} />
                }).collect_view()}
            </div>

            // Time control section
            <div class="px-6 py-8">
                <div class="flex items-center justify-between mb-8">
                    <h1 class="text-3xl font-bold tracking-tighter text-white">
                        "Quick match"
                    </h1>
                    <div class="flex flex-col items-end gap-1">
                        <div class="flex items-center rounded-lg bg-zinc-900 border border-zinc-800/60 p-0.5">
                            <button
                                on:click=move |_| set_rating_mode(RatingMode::Rated)
                                disabled=is_guest
                                class="px-3 py-1.5 text-xs font-semibold uppercase tracking-wide rounded-md transition-colors"
                                class:cursor-pointer=move || !is_guest()
                                class:cursor-not-allowed=is_guest
                                class:bg-white=move || rating_mode.get() == RatingMode::Rated && !is_guest()
                                class:text-zinc-950=move || rating_mode.get() == RatingMode::Rated && !is_guest()
                                class:text-zinc-600=is_guest
                                class:text-zinc-400=move || !(rating_mode.get() == RatingMode::Rated) && !is_guest()
                            >
                                "Rated"
                            </button>
                            <button
                                on:click=move |_| set_rating_mode(RatingMode::Casual)
                                class="px-3 py-1.5 text-xs font-semibold uppercase tracking-wide rounded-md transition-colors cursor-pointer"
                                class:bg-white=move || rating_mode.get() == RatingMode::Casual
                                class:text-zinc-950=move || rating_mode.get() == RatingMode::Casual
                                class:text-zinc-400=move || rating_mode.get() != RatingMode::Casual
                            >
                                "Casual"
                            </button>
                        </div>
                        <Show when=is_guest>
                            <span class="text-[11px] text-zinc-500">
                                "Sign in to play rated"
                            </span>
                        </Show>
                    </div>
                </div>

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
                                            rating_mode.get(),
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

            <RecentGames/>
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
