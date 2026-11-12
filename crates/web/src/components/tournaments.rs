use leptos::prelude::*;
use shared::{
    Category, NewTournament, TournamentFilter, TournamentFormat, TournamentStatus, TournamentSummary, TournamentVenue,
    ARENA_MAX_MINUTES, ARENA_MIN_MINUTES, SWISS_MAX_ROUNDS,
};

use crate::components::{use_current_user, ModalShell};
use crate::tournament::{create_tournament, list_tournaments, TournamentList};

/// A span of seconds as a countdown: `"12:34"`, `"2h 05m"`, `"3d 4h"`.
pub fn format_countdown(seconds: i64) -> String {
    let s = seconds.max(0);
    let (days, hours, minutes, secs) = (s / 86_400, (s % 86_400) / 3_600, (s % 3_600) / 60, s % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

/// The one-line "where is this tournament at" used by both the list rows
/// and the tournament page header. `now` is server-clock seconds.
pub fn status_line(t: &TournamentSummary, now: i64) -> String {
    match (t.status, t.format) {
        (TournamentStatus::Cancelled, _) => "Cancelled".to_string(),
        (TournamentStatus::Finished, _) => "Finished".to_string(),
        (TournamentStatus::Upcoming, TournamentFormat::Arena) => {
            format!("Starts in {}", format_countdown(t.starts_at - now))
        }
        (TournamentStatus::Live, TournamentFormat::Arena) => match t.ends_at() {
            Some(end) => format!("Live · {} left", format_countdown(end - now)),
            None => "Live".to_string(),
        },
        (TournamentStatus::Upcoming, _) => "Open for entries".to_string(),
        (TournamentStatus::Live, _) => match t.total_rounds {
            Some(total) if t.current_round > 0 => format!("Round {} of {total}", t.current_round),
            _ => "Starting".to_string(),
        },
    }
}

/// Small grey pill, for the format / time control / rated tags.
#[component]
pub fn Tag(#[prop(into)] label: String) -> impl IntoView {
    view! {
        <span class="px-2 py-0.5 rounded-control bg-zinc-800 border border-zinc-700/60 eyebrow-sm text-zinc-300 whitespace-nowrap">
            {label}
        </span>
    }
}

/// The tags that describe what kind of tournament this is.
#[component]
pub fn TournamentTags(summary: TournamentSummary) -> impl IntoView {
    view! {
        <div class="flex flex-wrap items-center gap-1.5">
            <Tag label=summary.format.label()/>
            <Tag label=summary.time_control_label()/>
            {(summary.venue == TournamentVenue::Otb).then(|| view! { <Tag label="Over the board"/> })}
            {(summary.venue == TournamentVenue::Online).then(|| view! {
                <Tag label={if summary.rated { "Rated" } else { "Casual" }}/>
            })}
            {summary.official.then(|| view! {
                <span class="px-2 py-0.5 rounded-control eyebrow-sm accent-text border accent-ring whitespace-nowrap">
                    "Official"
                </span>
            })}
        </div>
    }
}

#[component]
fn TournamentRowLink(summary: TournamentSummary, now: i64) -> impl IntoView {
    let href = format!("/tournament/{}", summary.id);
    let live = summary.status == TournamentStatus::Live;
    let line = status_line(&summary, now);
    let players = summary.player_count;
    let organizer = summary.organizer.clone();
    view! {
        <a href=href class="flex items-center justify-between gap-4 px-4 py-3.5 hover:bg-zinc-800/50 transition-colors">
            <div class="flex flex-col gap-1.5 min-w-0">
                <span class="text-sm font-semibold text-white truncate">{summary.name.clone()}</span>
                <TournamentTags summary=summary/>
            </div>
            <div class="flex flex-col items-end gap-1 flex-shrink-0 text-right">
                <span
                    class="text-sm font-semibold tabular-nums"
                    class:text-emerald-400=live
                    class:text-zinc-300=!live
                >
                    {line}
                </span>
                <span class="text-xs text-zinc-500">
                    {format!("{players} {}", if players == 1 { "player" } else { "players" })}
                    {organizer.map(|name| format!(" · by {name}"))}
                </span>
            </div>
        </a>
    }
}

#[component]
fn TournamentSection(
    title: &'static str,
    rows: Vec<TournamentSummary>,
    now: i64,
    /// Shown instead of hiding the section when there is nothing in it.
    #[prop(optional)]
    empty: Option<&'static str>,
) -> impl IntoView {
    if rows.is_empty() && empty.is_none() {
        return ().into_any();
    }
    view! {
        <section class="flex flex-col gap-3">
            <h2 class="eyebrow text-zinc-500">{title}</h2>
            {if rows.is_empty() {
                view! { <p class="text-zinc-500 text-sm italic px-1">{empty}</p> }.into_any()
            } else {
                view! {
                    <div class="flex flex-col divide-y divide-zinc-800 surface-card overflow-hidden">
                        {rows.into_iter().map(|t| view! { <TournamentRowLink summary=t now=now/> }).collect_view()}
                    </div>
                }.into_any()
            }}
        </section>
    }
    .into_any()
}

/// `(label, base seconds, increment seconds)`.
const TIME_CONTROLS: &[(&str, u32, u32)] = &[
    ("1+0", 60, 0),
    ("2+1", 120, 1),
    ("3+0", 180, 0),
    ("3+2", 180, 2),
    ("5+0", 300, 0),
    ("5+3", 300, 3),
    ("10+0", 600, 0),
    ("10+5", 600, 5),
    ("15+10", 900, 10),
    ("30+0", 1800, 0),
    ("60+30", 3600, 30),
    ("90+30", 5400, 30),
];
const DEFAULT_TIME_CONTROL: usize = 5;

/// `(label, minutes from now)`.
const START_OFFSETS: &[(&str, u32)] = &[
    ("In 5 minutes", 5),
    ("In 15 minutes", 15),
    ("In 30 minutes", 30),
    ("In 1 hour", 60),
    ("In 2 hours", 120),
    ("In 6 hours", 360),
    ("In 24 hours", 1440),
];

const ARENA_DURATIONS: &[u32] = &[20, 30, 45, 60, 90, 120];

const FIELD_CLASS: &str = "w-full px-3 py-2 rounded-control bg-zinc-950 border border-zinc-800 \
                           text-sm text-white placeholder-zinc-600 focus:outline-none focus:border-zinc-600";
const LABEL_CLASS: &str = "eyebrow-sm text-zinc-500";

/// One option of a segmented two/three-way choice.
#[component]
fn Choice<T>(
    value: T,
    current: RwSignal<T>,
    label: &'static str,
    #[prop(optional, into)] disabled: Signal<bool>,
) -> impl IntoView
where
    T: Copy + PartialEq + Send + Sync + 'static,
{
    let selected = move || current.get() == value;
    view! {
        <button
            type="button"
            on:click=move |_| if !disabled.get_untracked() { current.set(value) }
            disabled=move || disabled.get()
            class="flex-1 px-3 py-2 text-xs font-semibold uppercase tracking-wide rounded-control transition-colors"
            class:cursor-pointer=move || !disabled.get()
            class:cursor-not-allowed=move || disabled.get()
            class:bg-white=move || selected() && !disabled.get()
            class:text-zinc-950=move || selected() && !disabled.get()
            class:text-zinc-400=move || !selected() && !disabled.get()
            class:text-zinc-600=move || disabled.get()
        >
            {label}
        </button>
    }
}

#[component]
fn CreateTournamentModal(#[prop(into)] on_close: Callback<()>) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let venue = RwSignal::new(TournamentVenue::Online);
    let format = RwSignal::new(TournamentFormat::Arena);
    let time_control = RwSignal::new(DEFAULT_TIME_CONTROL);
    let starts_in = RwSignal::new(15u32);
    let duration = RwSignal::new(45u32);
    let rounds = RwSignal::new(5u32);
    let error = RwSignal::new(None::<String>);
    let submitting = RwSignal::new(false);

    let otb = Signal::derive(move || venue.get() == TournamentVenue::Otb);
    // An arena can't be run over the board, so picking that venue has to
    // move the format off it rather than leave an impossible combination
    // selected for the server to reject.
    Effect::new(move |_| {
        if otb.get() && format.get_untracked() == TournamentFormat::Arena {
            format.set(TournamentFormat::Swiss);
        }
    });

    let draft = move || {
        let (_, initial_seconds, increment_seconds) =
            TIME_CONTROLS[time_control.get_untracked().min(TIME_CONTROLS.len() - 1)];
        NewTournament {
            name: name.get_untracked(),
            format: format.get_untracked(),
            venue: venue.get_untracked(),
            initial_seconds,
            increment_seconds,
            // Only the official daily tournaments are rated; the server
            // rejects anything else.
            rated: false,
            starts_in_minutes: starts_in.get_untracked(),
            duration_minutes: duration.get_untracked(),
            rounds: rounds.get_untracked(),
        }
    };

    let navigate = leptos_router::hooks::use_navigate();
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if submitting.get_untracked() {
            return;
        }
        let new = draft();
        // Same check the server runs — this just answers without a round trip.
        if let Err(message) = new.validate() {
            error.set(Some(message.to_string()));
            return;
        }
        error.set(None);
        submitting.set(true);
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match create_tournament(new).await {
                Ok(id) => navigate(&format!("/tournament/{id}"), Default::default()),
                Err(e) => {
                    error.set(Some(server_message(&e)));
                    submitting.set(false);
                }
            }
        });
    };

    let select_u32 = move |target: RwSignal<u32>| {
        move |ev: leptos::ev::Event| {
            if let Ok(v) = event_target_value(&ev).parse() {
                target.set(v);
            }
        }
    };

    view! {
        <ModalShell labelled_by="create-tournament-title" on_close=on_close card_class="max-w-md p-6">
            <form on:submit=submit class="flex flex-col gap-4">
                <h2 id="create-tournament-title" class="text-lg font-bold text-white">
                    "New tournament"
                </h2>

                <label class="flex flex-col gap-1.5">
                    <span class=LABEL_CLASS>"Name"</span>
                    <input
                        type="text"
                        maxlength="60"
                        placeholder="Friday night blitz"
                        prop:value=name
                        on:input=move |ev| name.set(event_target_value(&ev))
                        class=FIELD_CLASS
                    />
                </label>

                <div class="flex flex-col gap-1.5">
                    <span class=LABEL_CLASS>"Where"</span>
                    <div class="flex items-center rounded-control bg-zinc-950 border border-zinc-800/60 p-0.5">
                        <Choice value=TournamentVenue::Online current=venue label="Online"/>
                        <Choice value=TournamentVenue::Otb current=venue label="Over the board"/>
                    </div>
                    <span class="text-xs text-zinc-500">
                        {move || if otb.get() {
                            "You enter the players and each result; the site does pairings and standings."
                        } else {
                            "Games are played here and results record themselves."
                        }}
                    </span>
                </div>

                <div class="flex flex-col gap-1.5">
                    <span class=LABEL_CLASS>"Format"</span>
                    <div class="flex items-center rounded-control bg-zinc-950 border border-zinc-800/60 p-0.5">
                        <Choice value=TournamentFormat::Arena current=format label="Arena" disabled=otb/>
                        <Choice value=TournamentFormat::Swiss current=format label="Swiss"/>
                        <Choice value=TournamentFormat::RoundRobin current=format label="Round robin"/>
                    </div>
                    <span class="text-xs text-zinc-500">
                        {move || match format.get() {
                            TournamentFormat::Arena => "Runs for a set time. Finish a game and you're paired again straight away.",
                            TournamentFormat::Swiss => "A set number of rounds, each pairing players on similar scores.",
                            TournamentFormat::RoundRobin => "Everyone plays everyone once. Up to 16 players.",
                        }}
                    </span>
                </div>

                <div class="grid grid-cols-2 gap-3">
                    <label class="flex flex-col gap-1.5">
                        <span class=LABEL_CLASS>"Time control"</span>
                        <select
                            class=FIELD_CLASS
                            on:change=move |ev| {
                                if let Ok(i) = event_target_value(&ev).parse() {
                                    time_control.set(i);
                                }
                            }
                        >
                            {TIME_CONTROLS.iter().enumerate().map(|(i, (label, _, _))| view! {
                                <option value=i.to_string() selected=i == DEFAULT_TIME_CONTROL>{*label}</option>
                            }).collect_view()}
                        </select>
                    </label>
                </div>

                <Show when=move || format.get() == TournamentFormat::Arena>
                    <div class="grid grid-cols-2 gap-3">
                        <label class="flex flex-col gap-1.5">
                            <span class=LABEL_CLASS>"Starts"</span>
                            <select class=FIELD_CLASS on:change=select_u32(starts_in)>
                                {START_OFFSETS.iter().map(|(label, minutes)| view! {
                                    <option value=minutes.to_string() selected=*minutes == 15>{*label}</option>
                                }).collect_view()}
                            </select>
                        </label>
                        <label class="flex flex-col gap-1.5">
                            <span class=LABEL_CLASS>"Runs for"</span>
                            <select class=FIELD_CLASS on:change=select_u32(duration)>
                                {ARENA_DURATIONS
                                    .iter()
                                    .filter(|m| (ARENA_MIN_MINUTES..=ARENA_MAX_MINUTES).contains(*m))
                                    .map(|minutes| view! {
                                        <option value=minutes.to_string() selected=*minutes == 45>
                                            {format!("{minutes} minutes")}
                                        </option>
                                    }).collect_view()}
                            </select>
                        </label>
                    </div>
                </Show>

                <Show when=move || format.get() == TournamentFormat::Swiss>
                    <label class="flex flex-col gap-1.5">
                        <span class=LABEL_CLASS>"Rounds"</span>
                        <select class=FIELD_CLASS on:change=select_u32(rounds)>
                            {(1..=SWISS_MAX_ROUNDS).map(|n| view! {
                                <option value=n.to_string() selected=n == 5>{n.to_string()}</option>
                            }).collect_view()}
                        </select>
                    </label>
                </Show>

                {move || error.get().map(|message| view! {
                    <p class="text-xs text-red-400" role="alert">{message}</p>
                })}

                <div class="flex items-center justify-end gap-3 pt-1">
                    <button
                        type="button"
                        on:click=move |_| on_close.run(())
                        class="px-4 py-2 text-sm text-zinc-400 hover:text-white transition-colors cursor-pointer"
                    >
                        "Cancel"
                    </button>
                    <button
                        type="submit"
                        disabled=move || submitting.get()
                        class="btn-primary px-5 py-2 text-sm font-semibold cursor-pointer disabled:opacity-60"
                    >
                        {move || if submitting.get() { "Creating…" } else { "Create" }}
                    </button>
                </div>
            </form>
        </ModalShell>
    }
}

/// The human half of a `ServerFnError`. Its `Display` prefixes every message
/// with "error running server function: ", which is noise in a form.
pub fn server_message(e: &ServerFnError) -> String {
    match e {
        ServerFnError::ServerError(message) => message.clone(),
        other => other.to_string(),
    }
}

const SELECT_CLASS: &str = "px-3 py-2 rounded-control bg-zinc-950 border border-zinc-800 text-sm text-zinc-300 \
                            focus:outline-none focus:border-zinc-600 cursor-pointer";

/// The list's search box and dropdowns. Held in signals by the caller, so
/// narrowing the list never refetches it.
#[component]
fn Filters(
    query: RwSignal<String>,
    venue: RwSignal<String>,
    format: RwSignal<String>,
    speed: RwSignal<String>,
    rated: RwSignal<String>,
) -> impl IntoView {
    let select = |signal: RwSignal<String>, label: &'static str, options: &'static [(&'static str, &'static str)]| {
        view! {
            <select
                aria-label=label
                class=SELECT_CLASS
                prop:value=signal
                on:change=move |ev| signal.set(event_target_value(&ev))
            >
                {options.iter().map(|(value, text)| view! { <option value=*value>{*text}</option> }).collect_view()}
            </select>
        }
    };
    let active = move || {
        !query.get().trim().is_empty()
            || [venue, format, speed, rated].iter().any(|s| !s.get().is_empty())
    };
    let clear = move |_| {
        query.set(String::new());
        for s in [venue, format, speed, rated] {
            s.set(String::new());
        }
    };
    view! {
        <div class="flex flex-wrap items-center gap-2">
            <input
                type="search"
                placeholder="Search by name or organizer…"
                aria-label="Search tournaments"
                prop:value=query
                on:input=move |ev| query.set(event_target_value(&ev))
                class="flex-1 min-w-[12rem] px-3 py-2 rounded-control bg-zinc-950 border border-zinc-800 text-sm
                       text-white placeholder-zinc-600 focus:outline-none focus:border-zinc-600"
            />
            {select(venue, "Venue", &[("", "Online & OTB"), ("online", "Online"), ("otb", "Over the board")])}
            {select(format, "Format", &[("", "Any format"), ("arena", "Arena"), ("swiss", "Swiss"), ("round_robin", "Round robin")])}
            {select(speed, "Speed", &[("", "Any speed"), ("bullet", "Bullet"), ("blitz", "Blitz"), ("rapid", "Rapid"), ("classical", "Classical")])}
            {select(rated, "Rating", &[("", "Rated & casual"), ("rated", "Rated"), ("casual", "Casual")])}
            <Show when=active>
                <button
                    on:click=clear
                    class="px-3 py-2 text-xs font-medium text-zinc-400 hover:text-white transition-colors cursor-pointer"
                >
                    "Clear"
                </button>
            </Show>
        </div>
    }
}

/// The dropdowns' values as a filter.
fn build_filter(query: &str, venue: &str, format: &str, speed: &str, rated: &str) -> TournamentFilter {
    TournamentFilter {
        query: query.to_string(),
        venue: TournamentVenue::parse(venue),
        format: TournamentFormat::parse(format),
        speed: match speed {
            "bullet" => Some(Category::Bullet),
            "blitz" => Some(Category::Blitz),
            "rapid" => Some(Category::Rapid),
            "classical" => Some(Category::Classical),
            _ => None,
        },
        rated: match rated {
            "rated" => Some(true),
            "casual" => Some(false),
            _ => None,
        },
    }
}

/// The `/tournaments` page body.
#[component]
pub fn TournamentBrowser() -> impl IntoView {
    let list = Resource::new(|| (), |_| async move { list_tournaments().await });
    let creating = RwSignal::new(false);
    let (query, venue, format, speed, rated) = (
        RwSignal::new(String::new()),
        RwSignal::new(String::new()),
        RwSignal::new(String::new()),
        RwSignal::new(String::new()),
        RwSignal::new(String::new()),
    );
    let user = use_current_user();
    let can_organize = move || {
        user.get()
            .and_then(|r| r.ok())
            .flatten()
            .is_some_and(|u| !u.is_guest && u.username.is_some())
    };

    let fallback = || {
        view! {
            <div class="flex flex-col gap-2">
                {(0..4).map(|_| view! { <div class="h-16 rounded-card skeleton-shimmer"/> }).collect_view()}
            </div>
        }
    };

    view! {
        <div class="px-6 py-8 max-w-3xl mx-auto flex flex-col gap-8">
            <div class="flex items-end justify-between gap-4">
                <div class="flex flex-col gap-1">
                    <h1 class="display-1 text-white">"Tournaments"</h1>
                    <p class="text-sm text-zinc-400">
                        "Daily arenas, or run your own — online or over the board."
                    </p>
                </div>
                <Transition fallback=|| ()>
                    <Show when=can_organize>
                        <button
                            on:click=move |_| creating.set(true)
                            class="btn-primary px-4 py-2 text-sm font-semibold cursor-pointer flex-shrink-0"
                        >
                            "New tournament"
                        </button>
                    </Show>
                </Transition>
            </div>

            <Filters query=query venue=venue format=format speed=speed rated=rated/>

            <Transition fallback=fallback>
                {move || list.get().map(|result| match result {
                    Err(e) => view! {
                        <p class="text-zinc-500 text-sm">"Couldn't load tournaments: " {server_message(&e)}</p>
                    }.into_any(),
                    Ok(TournamentList { tournaments, server_now }) => {
                        let filter = build_filter(&query.get(), &venue.get(), &format.get(), &speed.get(), &rated.get());
                        let total = tournaments.len();
                        let tournaments: Vec<_> = tournaments.into_iter().filter(|t| filter.matches(t)).collect();
                        if tournaments.is_empty() && total > 0 {
                            return view! {
                                <p class="text-zinc-500 text-sm italic px-1">"No tournaments match those filters."</p>
                            }.into_any();
                        }
                        let open = |t: &TournamentSummary| !t.status.is_over();
                        let mine: Vec<_> = tournaments.iter().filter(|t| t.mine && open(t)).cloned().collect();
                        let live: Vec<_> = tournaments
                            .iter()
                            .filter(|t| !t.mine && t.status == TournamentStatus::Live)
                            .cloned()
                            .collect();
                        let upcoming: Vec<_> = tournaments
                            .iter()
                            .filter(|t| !t.mine && t.status == TournamentStatus::Upcoming)
                            .cloned()
                            .collect();
                        let finished: Vec<_> = tournaments
                            .iter()
                            .filter(|t| t.status == TournamentStatus::Finished)
                            .take(10)
                            .cloned()
                            .collect();
                        view! {
                            <div class="flex flex-col gap-8">
                                <TournamentSection title="Yours" rows=mine now=server_now/>
                                <TournamentSection title="Live now" rows=live now=server_now/>
                                <TournamentSection
                                    title="Coming up"
                                    rows=upcoming
                                    now=server_now
                                    empty="Nothing scheduled right now."
                                />
                                <TournamentSection title="Recently finished" rows=finished now=server_now/>
                            </div>
                        }.into_any()
                    }
                })}
            </Transition>
        </div>

        <Show when=move || creating.get()>
            <CreateTournamentModal on_close=move |_| creating.set(false)/>
        </Show>
    }
}

/// Root-owned for the same reason as `MyRatingsResource` — see `HomePage`.
#[derive(Copy, Clone)]
pub struct HomeTournamentsResource(pub Resource<Option<TournamentList>>);

/// Call once in the home route.
pub fn provide_home_tournaments() {
    provide_context(HomeTournamentsResource(Resource::new(
        || (),
        |_| async move { list_tournaments().await.ok() },
    )));
}

/// How many tournaments the home page advertises.
const TEASER_LIMIT: usize = 3;

/// The home page's "what's on" strip: whatever is live or next, with the
/// viewer's own tournaments first.
#[component]
pub fn TournamentTeaser() -> impl IntoView {
    let list = use_context::<HomeTournamentsResource>()
        .expect("provide_home_tournaments must be called in the home route")
        .0;

    view! {
        <Transition fallback=|| ()>
            {move || {
                let TournamentList { tournaments, server_now } = list.get().flatten()?;
                let mut open: Vec<TournamentSummary> = tournaments
                    .into_iter()
                    .filter(|t| !t.status.is_over() && (t.official || t.mine || t.status == TournamentStatus::Live))
                    .collect();
                // Stable, so the server's own ordering (live first, then by
                // start time) survives within each group.
                open.sort_by_key(|t| !t.mine);
                open.truncate(TEASER_LIMIT);
                if open.is_empty() {
                    return None;
                }
                Some(view! {
                    <div class="flex flex-col gap-3">
                        <div class="flex items-baseline justify-between">
                            <h2 class="eyebrow text-zinc-500">"Tournaments"</h2>
                            <a href="/tournaments" class="text-xs text-zinc-400 hover:text-white transition-colors">
                                "All tournaments \u{2192}"
                            </a>
                        </div>
                        <div class="flex flex-col divide-y divide-zinc-800 surface-card overflow-hidden">
                            {open.into_iter().map(|t| view! { <TournamentRowLink summary=t now=server_now/> }).collect_view()}
                        </div>
                    </div>
                })
            }}
        </Transition>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn countdowns_pick_the_right_scale() {
        assert_eq!(format_countdown(-5), "0:00");
        assert_eq!(format_countdown(59), "0:59");
        assert_eq!(format_countdown(754), "12:34");
        assert_eq!(format_countdown(2 * 3600 + 5 * 60), "2h 05m");
        assert_eq!(format_countdown(3 * 86_400 + 4 * 3600), "3d 4h");
    }

    fn summary(format: TournamentFormat, status: TournamentStatus) -> TournamentSummary {
        TournamentSummary {
            id: Uuid::nil(),
            name: "T".into(),
            format,
            venue: TournamentVenue::Online,
            status,
            initial_seconds: 180,
            increment_seconds: 2,
            rated: true,
            starts_at: 1_000,
            duration_minutes: Some(30),
            total_rounds: Some(5),
            current_round: 2,
            player_count: 4,
            organizer: None,
            official: false,
            mine: false,
        }
    }

    #[test]
    fn status_lines() {
        use TournamentFormat::*;
        use TournamentStatus::*;
        assert_eq!(status_line(&summary(Arena, Upcoming), 940), "Starts in 1:00");
        assert_eq!(status_line(&summary(Arena, Live), 1_000 + 29 * 60), "Live · 1:00 left");
        assert_eq!(status_line(&summary(Swiss, Upcoming), 0), "Open for entries");
        assert_eq!(status_line(&summary(Swiss, Live), 0), "Round 2 of 5");
        assert_eq!(status_line(&summary(RoundRobin, Finished), 0), "Finished");
    }
}
