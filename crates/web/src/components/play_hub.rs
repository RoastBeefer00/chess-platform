use leptos::prelude::*;
use shared::{RatingMode, TimeControl, TimeMode};

use crate::components::{use_current_user, EloCardRow, MatchmakingModal, RecentGames, ResumeGame};

#[cfg(feature = "hydrate")]
const RATING_MODE_STORAGE_KEY: &str = "gambit:rating_mode";
/// Last time control played, as `"<initial_ms>:<increment_ms>"`. Drives the
/// primary card below so the fastest path is the one you actually use.
#[cfg(feature = "hydrate")]
const LAST_TC_STORAGE_KEY: &str = "gambit:last_tc";

/// `(label, initial_ms, increment_ms)`.
type Preset = (&'static str, i64, i64);

const BULLET: &[Preset] = &[("1 + 0", 60_000, 0), ("1 + 1", 60_000, 1_000), ("2 + 1", 120_000, 1_000)];
const BLITZ: &[Preset] = &[
    ("3 + 0", 180_000, 0),
    ("3 + 2", 180_000, 2_000),
    ("5 + 0", 300_000, 0),
    ("5 + 5", 300_000, 5_000),
];
const RAPID: &[Preset] = &[("10 + 0", 600_000, 0), ("15 + 10", 900_000, 10_000)];

const GROUPS: &[(&str, &[Preset])] = &[("Bullet", BULLET), ("Blitz", BLITZ), ("Rapid", RAPID)];

/// Default primary control for someone who hasn't played yet. 5+0 is the
/// median choice on every site that publishes the numbers.
const DEFAULT_PRESET: Preset = ("5 + 0", 300_000, 0);

#[cfg(feature = "hydrate")]
fn preset_for(initial: i64, increment: i64) -> Option<Preset> {
    GROUPS
        .iter()
        .flat_map(|(_, presets)| presets.iter())
        .find(|(_, i, inc)| *i == initial && *inc == increment)
        .copied()
}

fn category_of(initial_ms: i64) -> &'static str {
    match initial_ms {
        ms if ms < 180_000 => "Bullet",
        ms if ms < 600_000 => "Blitz",
        _ => "Rapid",
    }
}

#[component]
pub fn PlayHub() -> impl IntoView {
    let searching = RwSignal::new(None::<(TimeControl, RatingMode)>);
    let rating_mode = RwSignal::new(RatingMode::Rated);
    let primary = RwSignal::new(DEFAULT_PRESET);
    let user = use_current_user();
    let is_guest = move || {
        user.get()
            .and_then(|r| r.ok())
            .flatten()
            .is_some_and(|u| u.is_guest)
    };

    // Restore both remembered choices client-side only (post-hydration),
    // never during render — the server has no `window`, so a render-time read
    // would make SSR and the first client paint disagree.
    Effect::new(move |_| {
        if is_guest() {
            rating_mode.set(RatingMode::Casual);
        }
        #[cfg(feature = "hydrate")]
        if let Some(store) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            if !is_guest() {
                if let Ok(Some(saved)) = store.get_item(RATING_MODE_STORAGE_KEY) {
                    if saved == "casual" {
                        rating_mode.set(RatingMode::Casual);
                    }
                }
            }
            if let Ok(Some(saved)) = store.get_item(LAST_TC_STORAGE_KEY) {
                if let Some((initial, increment)) = saved.split_once(':') {
                    if let (Ok(i), Ok(inc)) = (initial.parse::<i64>(), increment.parse::<i64>()) {
                        if let Some(preset) = preset_for(i, inc) {
                            primary.set(preset);
                        }
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
        #[cfg(feature = "hydrate")]
        if let Some(store) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = store.set_item(RATING_MODE_STORAGE_KEY, &mode.to_string());
        }
    };

    let start = move |initial: i64, increment: i64| {
        #[cfg(feature = "hydrate")]
        if let Some(store) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = store.set_item(LAST_TC_STORAGE_KEY, &format!("{initial}:{increment}"));
        }
        searching.set(Some((
            TimeControl { initial_time: initial, mode: TimeMode::Increment(increment) },
            rating_mode.get(),
        )));
    };

    let mode_btn = move |mode: RatingMode, label: &'static str, disabled: bool| {
        view! {
            <button
                on:click=move |_| set_rating_mode(mode)
                disabled=disabled
                class="px-3 py-1.5 text-xs font-semibold uppercase tracking-wide rounded-control transition-colors"
                class:cursor-pointer=move || !disabled
                class:cursor-not-allowed=move || disabled
                class:bg-white=move || rating_mode.get() == mode && !disabled
                class:text-zinc-950=move || rating_mode.get() == mode && !disabled
                class:text-zinc-600=move || disabled
                class:text-zinc-400=move || rating_mode.get() != mode && !disabled
            >
                {label}
            </button>
        }
    };

    view! {
        <div class="flex flex-col max-w-2xl mx-auto px-6 pb-12">
            <div class="pt-6">
                <ResumeGame/>
            </div>

            // Ratings strip.
            <div class="flex gap-3 overflow-x-auto scrollbar-none pt-6 pb-2">
                <EloCardRow/>
            </div>

            // ── Primary action ────────────────────────────────────────────
            // One obvious path in, rather than a flat wrap of a dozen
            // identically-weighted pills with no starting point.
            <div class="surface-card mt-6 p-6 flex flex-col gap-5">
                <div class="flex items-start justify-between gap-4">
                    <div class="flex flex-col gap-1 min-w-0">
                        <span class="eyebrow text-zinc-500">
                            {move || category_of(primary.get().1)}
                        </span>
                        <span class="display-1 text-white">{move || primary.get().0}</span>
                    </div>
                    <div class="flex flex-col items-end gap-1.5">
                        <div class="flex items-center rounded-control bg-zinc-950 border border-zinc-800/60 p-0.5">
                            {move || mode_btn(RatingMode::Rated, "Rated", is_guest())}
                            {mode_btn(RatingMode::Casual, "Casual", false)}
                        </div>
                        <Show when=is_guest>
                            <span class="text-[11px] text-zinc-500">"Sign in to play rated"</span>
                        </Show>
                    </div>
                </div>
                <button
                    on:click=move |_| { let p = primary.get(); start(p.1, p.2); }
                    class="btn-primary w-full py-3.5 text-base font-bold tracking-tight cursor-pointer
                           active:scale-[0.99] transition-transform"
                >
                    "Play"
                </button>
            </div>

            // ── Secondary: everything else, grouped ───────────────────────
            <div class="mt-8 flex flex-col gap-5">
                <span class="eyebrow text-zinc-500">
                    "More time controls"
                </span>
                {GROUPS.iter().map(|(label, presets)| view! {
                    <div class="flex items-baseline gap-4">
                        <span class="w-14 flex-shrink-0 text-xs font-semibold text-zinc-500">{*label}</span>
                        <div class="flex flex-wrap gap-2">
                            {presets.iter().map(|&(name, ms, inc)| view! {
                                <button
                                    on:click=move |_| start(ms, inc)
                                    class="px-3.5 py-2 rounded-control bg-zinc-900 border border-zinc-800
                                           text-sm font-semibold text-zinc-200
                                           hover:bg-zinc-800 hover:border-zinc-700 hover:text-white
                                           active:scale-[0.97] transition-all duration-150 cursor-pointer"
                                >
                                    {name}
                                </button>
                            }).collect_view()}
                        </div>
                    </div>
                }).collect_view()}
            </div>

            <div class="mt-10">
                <RecentGames/>
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
