use std::collections::HashSet;
use std::future::Future;

use leptos::prelude::*;
use leptos_meta::Title;
use shared::{
    format_score2, PairingResult, PairingView, StandingRow, TournamentFormat, TournamentStatus,
    tiebreak_group, TournamentSummary, TournamentVenue, TournamentView, TournamentViewer,
};
use uuid::Uuid;

use crate::components::tournaments::{server_message, status_line, TournamentTags};
use crate::friends::get_friends_overview;
use crate::tournament::{
    add_tiebreak_round, invite_to_tournament, add_tournament_player, cancel_tournament, finish_tournament, join_tournament,
    next_tournament_round, remove_tournament_player, set_pairing_result, start_tournament,
    withdraw_from_tournament,
};

/// How often the page re-reads the tournament, in seconds. `None` once
/// nothing can change any more.
///
/// The live-arena figure is load-bearing, not cosmetic: the poll is what
/// keeps an entrant in the pairing pool, and the server drops anyone it
/// hasn't heard from in `ARENA_LOBBY_WINDOW_SECS` (12s). Three polls fit
/// inside that window, so one dropped request doesn't cost a pairing.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
fn poll_seconds(summary: &TournamentSummary, now: i64) -> Option<u32> {
    match (summary.status, summary.format, summary.venue) {
        (TournamentStatus::Finished | TournamentStatus::Cancelled, _, _) => None,
        (TournamentStatus::Live, TournamentFormat::Arena, _) => Some(4),
        (TournamentStatus::Live, _, TournamentVenue::Online) => Some(5),
        // Only the organizer changes an over-the-board event, and their own
        // clicks refresh immediately. This is for everyone else in the room.
        (TournamentStatus::Live, _, TournamentVenue::Otb) => Some(15),
        // Tighten up as an arena's start approaches so it opens on time.
        (TournamentStatus::Upcoming, TournamentFormat::Arena, _) => {
            Some(if summary.starts_at - now <= 120 { 4 } else { 15 })
        }
        (TournamentStatus::Upcoming, _, _) => Some(10),
    }
}

/// Everything an action button needs: where to report a failure, and how to
/// pull fresh state once it succeeds.
#[derive(Clone, Copy)]
struct Actions {
    error: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    refresh: Callback<()>,
}

impl Actions {
    /// Runs one server call, then refreshes — on failure too, since a
    /// rejected action usually means the page was showing stale state.
    /// One at a time: a second click while the first is in flight is
    /// dropped rather than queued.
    ///
    /// Only ever reached from an event handler, so only ever in the browser.
    fn run<F>(self, call: F)
    where
        F: Future<Output = Result<(), ServerFnError>> + 'static,
    {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        leptos::task::spawn_local(async move {
            match call.await {
                Ok(()) => self.error.set(None),
                Err(e) => self.error.set(Some(server_message(&e))),
            }
            self.busy.set(false);
            self.refresh.run(());
        });
    }
}

const BTN_PRIMARY: &str = "btn-primary px-4 py-2 text-sm font-semibold cursor-pointer disabled:opacity-60 whitespace-nowrap";
const BTN_SECONDARY: &str = "px-4 py-2 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-control \
                             hover:border-zinc-500 hover:text-white transition-colors cursor-pointer \
                             disabled:opacity-60 whitespace-nowrap";

/// Players tied for a podium place, best-first; empty when the podium is clear.
fn tied_for_podium(standings: &[StandingRow]) -> Vec<Uuid> {
    let ranked: Vec<(Uuid, u32)> =
        standings.iter().filter(|r| !r.withdrawn).map(|r| (r.player_id, r.score2)).collect();
    tiebreak_group(&ranked)
}

/// What the viewer themselves can do: enter, leave, or be told why not.
#[component]
fn EntryControls(
    id: Uuid,
    summary: Memo<TournamentSummary>,
    viewer: Memo<TournamentViewer>,
    actions: Actions,
) -> impl IntoView {
    move || {
        let s = summary.get();
        let v = viewer.get();
        if s.status.is_over() || s.venue == TournamentVenue::Otb {
            return ().into_any();
        }
        let entered = v.player_id.is_some() && !v.withdrawn;
        let arena = s.format == TournamentFormat::Arena;
        // A round-based event locks its field when it starts; an arena is a
        // drop-in until it ends.
        let open = arena || s.status == TournamentStatus::Upcoming;

        if entered {
            let waiting = arena && s.status == TournamentStatus::Live;
            return view! {
                <div class="flex flex-wrap items-center gap-3">
                    {waiting.then(|| view! {
                        <span class="inline-flex items-center gap-2 text-sm text-emerald-400">
                            <span class="relative flex w-2 h-2">
                                <span class="absolute inline-flex w-full h-full rounded-full bg-emerald-500 opacity-60 animate-ping"></span>
                                <span class="relative inline-flex w-2 h-2 rounded-full bg-emerald-500"></span>
                            </span>
                            "Finding you an opponent — stay on this page"
                        </span>
                    })}
                    {(!waiting).then(|| view! {
                        <span class="text-sm text-zinc-400">"You're in."</span>
                    })}
                    <button
                        class=BTN_SECONDARY
                        disabled=move || actions.busy.get()
                        on:click=move |_| actions.run(withdraw_from_tournament(id))
                    >
                        {if s.status == TournamentStatus::Upcoming { "Leave" } else { "Withdraw" }}
                    </button>
                </div>
            }
            .into_any();
        }

        if !open {
            return view! {
                <span class="text-sm text-zinc-500">"Entries closed when the first round started."</span>
            }
            .into_any();
        }
        if !v.signed_in {
            return view! { <a href="/login" class=BTN_PRIMARY>"Sign in to join"</a> }.into_any();
        }
        if !v.can_enter {
            return view! {
                <a href="/login" class=BTN_SECONDARY>"Create an account to join"</a>
            }
            .into_any();
        }
        view! {
            <button
                class=BTN_PRIMARY
                disabled=move || actions.busy.get()
                on:click=move |_| actions.run(join_tournament(id))
            >
                {if v.withdrawn { "Rejoin" } else { "Join" }}
            </button>
        }
        .into_any()
    }
}

/// "Invite friends": copy the link, or nudge an online friend directly.
///
/// Mounted only while open, so the friends list is fetched on demand and
/// never for someone just reading the page.
#[component]
fn InvitePanel(id: Uuid, actions: Actions) -> impl IntoView {
    let open = RwSignal::new(false);
    let copied = RwSignal::new(false);
    let sent = RwSignal::new(HashSet::<Uuid>::new());

    let copy_link = move |_| {
        #[cfg(feature = "hydrate")]
        if let Some(win) = web_sys::window() {
            let origin = win.location().origin().unwrap_or_default();
            let _ = win.navigator().clipboard().write_text(&format!("{origin}/tournament/{id}"));
            copied.set(true);
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(2_000).await;
                copied.set(false);
            });
        }
    };

    let invite = move |friend: Uuid| {
        sent.update(|s| {
            s.insert(friend);
        });
        leptos::task::spawn_local(async move {
            match invite_to_tournament(id, friend).await {
                Ok(()) => actions.error.set(None),
                Err(e) => {
                    sent.update(|s| {
                        s.remove(&friend);
                    });
                    actions.error.set(Some(server_message(&e)));
                }
            }
        });
    };

    view! {
        <div class="flex flex-col gap-3">
            <div class="flex flex-wrap items-center gap-3">
                <button class=BTN_SECONDARY on:click=move |_| open.update(|o| *o = !*o)>
                    {move || if open.get() { "Hide invites" } else { "Invite friends" }}
                </button>
                <button class=BTN_SECONDARY on:click=copy_link>
                    {move || if copied.get() { "Link copied" } else { "Copy link" }}
                </button>
            </div>
            <Show when=move || open.get()>
                {
                    let friends = Resource::new(|| (), |_| async move { get_friends_overview().await });
                    view! {
                        <Transition fallback=|| view! { <div class="h-16 rounded-card skeleton-shimmer"/> }>
                            {move || friends.get().map(|result| match result {
                                Err(e) => view! {
                                    <p class="text-sm text-zinc-500">"Couldn't load friends: " {server_message(&e)}</p>
                                }.into_any(),
                                Ok(overview) if overview.friends.is_empty() => view! {
                                    <p class="text-sm text-zinc-500 italic">
                                        "No friends to invite yet — copy the link and send it instead."
                                    </p>
                                }.into_any(),
                                Ok(overview) => {
                                    let mut rows = overview.friends;
                                    rows.sort_by_key(|r| !r.online);
                                    view! {
                                        <div class="surface-card overflow-hidden divide-y divide-zinc-800 max-h-64 overflow-y-auto">
                                            {rows.into_iter().map(|row| {
                                                let friend = row.user.id;
                                                let online = row.online;
                                                let name = row.user.username.clone().unwrap_or_else(|| "Anonymous".to_string());
                                                view! {
                                                    <div class="flex items-center justify-between gap-3 px-4 py-2.5">
                                                        <span class="flex items-center gap-2 min-w-0 text-sm text-white">
                                                            <span
                                                                class="w-2 h-2 rounded-full flex-shrink-0"
                                                                class:bg-emerald-500=online
                                                                class:bg-zinc-600=!online
                                                            ></span>
                                                            <span class="truncate">{name}</span>
                                                        </span>
                                                        {move || if sent.get().contains(&friend) {
                                                            view! { <span class="text-xs text-zinc-500">"Invited"</span> }.into_any()
                                                        } else if online {
                                                            view! {
                                                                <button
                                                                    class="px-3 py-1 text-xs font-semibold text-zinc-300 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                                                                    on:click=move |_| invite(friend)
                                                                >
                                                                    "Invite"
                                                                </button>
                                                            }.into_any()
                                                        } else {
                                                            view! { <span class="text-xs text-zinc-600">"Offline"</span> }.into_any()
                                                        }}
                                                    </div>
                                                }
                                            }).collect_view()}
                                        </div>
                                    }.into_any()
                                }
                            })}
                        </Transition>
                    }
                }
            </Show>
        </div>
    }
}

/// The organizer's controls: start, advance, finish, cancel.
#[component]
fn OrganizerControls(
    id: Uuid,
    summary: Memo<TournamentSummary>,
    pairings: Memo<Vec<PairingView>>,
    standings: Memo<Vec<StandingRow>>,
    actions: Actions,
) -> impl IntoView {
    move || {
        let s = summary.get();
        let round_based = s.format != TournamentFormat::Arena;
        let busy = move || actions.busy.get();
        // Named rather than inlined into `disabled=`: a bare `<` or `>` in
        // an attribute value is ambiguous with the tag's own brackets.
        let too_few = s.player_count < 2;

        match s.status {
            TournamentStatus::Upcoming => view! {
                <div class="flex flex-wrap items-center gap-3">
                    {round_based.then(|| view! {
                        <button
                            class=BTN_PRIMARY
                            disabled=move || busy() || too_few
                            on:click=move |_| actions.run(start_tournament(id))
                        >
                            "Start tournament"
                        </button>
                    })}
                    {(round_based && too_few).then(|| view! {
                        <span class="text-xs text-zinc-500">"Needs at least two players."</span>
                    })}
                    <button class=BTN_SECONDARY disabled=busy on:click=move |_| actions.run(cancel_tournament(id))>
                        "Cancel tournament"
                    </button>
                </div>
            }
            .into_any(),
            TournamentStatus::Live if round_based => {
                let undecided = pairings
                    .get()
                    .iter()
                    .filter(|p| p.round == s.current_round && p.result.is_none())
                    .count();
                let pending = undecided > 0;
                let last_round = s.total_rounds.is_some_and(|total| s.current_round >= total);
                // Offered once the last round is in and the podium is still
                // level somewhere; the server works out who plays off.
                let can_tiebreak = last_round
                    && !pending
                    && s.venue == TournamentVenue::Otb
                    && !tied_for_podium(&standings.get()).is_empty();
                let next_label = if last_round {
                    "Finish tournament".to_string()
                } else {
                    format!("Start round {}", s.current_round + 1)
                };
                view! {
                    <div class="flex flex-wrap items-center gap-3">
                        <button
                            class=BTN_PRIMARY
                            disabled=move || busy() || pending
                            on:click=move |_| actions.run(next_tournament_round(id))
                        >
                            {next_label}
                        </button>
                        {pending.then(|| view! {
                            <span class="text-xs text-zinc-500">
                                {format!(
                                    "{undecided} {} of round {} still to finish.",
                                    if undecided == 1 { "game" } else { "games" },
                                    s.current_round,
                                )}
                            </span>
                        })}
                        {can_tiebreak.then(|| view! {
                            <button
                                class=BTN_SECONDARY
                                disabled=busy
                                on:click=move |_| actions.run(add_tiebreak_round(id))
                            >
                                "Add tie-break round"
                            </button>
                        })}
                        {(!last_round).then(|| view! {
                            <button
                                class=BTN_SECONDARY
                                disabled=busy
                                on:click=move |_| actions.run(finish_tournament(id))
                            >
                                "End early"
                            </button>
                        })}
                    </div>
                }
                .into_any()
            }
            _ => ().into_any(),
        }
    }
}

/// Organizer-only, over the board: type a name, press enter, repeat.
///
/// Its own component, outside anything that re-renders on a poll, so the
/// text box keeps its contents and its focus while the organizer works down
/// a sign-up sheet.
#[component]
fn AddPlayerForm(id: Uuid, actions: Actions) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let value = name.get_untracked();
        // Don't swallow the name if an earlier add is still in flight.
        if value.trim().is_empty() || actions.busy.get_untracked() {
            return;
        }
        name.set(String::new());
        actions.run(add_tournament_player(id, value));
    };
    view! {
        <form on:submit=submit class="flex flex-col gap-1.5">
            <div class="flex items-center gap-2">
                <input
                    type="text"
                    maxlength="40"
                    placeholder="Add a player…"
                    aria-label="Player name"
                    prop:value=name
                    on:input=move |ev| name.set(event_target_value(&ev))
                    class="flex-1 min-w-0 px-3 py-2 rounded-control bg-zinc-950 border border-zinc-800 text-sm
                           text-white placeholder-zinc-600 focus:outline-none focus:border-zinc-600"
                />
                <button type="submit" class=BTN_SECONDARY>"Add"</button>
            </div>
            <span class="text-xs text-zinc-500">
                "A name for someone without an account, or @username to enter one."
            </span>
        </form>
    }
}

#[component]
fn Standings(
    id: Uuid,
    summary: Memo<TournamentSummary>,
    standings: Memo<Vec<StandingRow>>,
    viewer: Memo<TournamentViewer>,
    actions: Actions,
) -> impl IntoView {
    view! {
        <section class="flex flex-col gap-3">
            <h2 class="eyebrow text-zinc-500">
                {move || if summary.get().status == TournamentStatus::Upcoming { "Players" } else { "Standings" }}
            </h2>
            {move || {
                let rows = standings.get();
                let s = summary.get();
                let v = viewer.get();
                if rows.is_empty() {
                    return view! {
                        <p class="text-zinc-500 text-sm italic px-1">"Nobody has entered yet."</p>
                    }.into_any();
                }
                let started = s.status != TournamentStatus::Upcoming;
                // Buchholz only means something once rounds are being paired
                // on score; in an arena it would just be a confusing number.
                let show_tiebreak = started && s.format != TournamentFormat::Arena;
                let can_manage = v.is_organizer && !s.status.is_over() && s.venue == TournamentVenue::Otb;
                view! {
                    <div class="surface-card overflow-hidden">
                        <table class="w-full text-sm">
                            <thead>
                                <tr class="text-left eyebrow-sm text-zinc-500 border-b border-zinc-800">
                                    <th class="px-3 py-2 w-10 font-semibold">"#"</th>
                                    <th class="px-3 py-2 font-semibold">"Player"</th>
                                    {started.then(|| view! {
                                        <th class="px-3 py-2 text-right font-semibold" title="Wins / draws / losses">"W/D/L"</th>
                                    })}
                                    {show_tiebreak.then(|| view! {
                                        <th class="px-3 py-2 text-right font-semibold" title="Buchholz: the combined score of the opponents played">"TB"</th>
                                    })}
                                    {started.then(|| view! {
                                        <th class="px-3 py-2 text-right font-semibold">"Pts"</th>
                                    })}
                                    {can_manage.then(|| view! { <th class="w-8"></th> })}
                                </tr>
                            </thead>
                            <tbody class="divide-y divide-zinc-800">
                                {rows.into_iter().map(|row| {
                                    let is_me = v.player_id == Some(row.player_id);
                                    let player_id = row.player_id;
                                    let withdrawn = row.withdrawn;
                                    let name = match row.username.clone() {
                                        Some(username) => view! {
                                            <a href=format!("/u/{username}") class="hover:underline">{row.name.clone()}</a>
                                        }.into_any(),
                                        None => view! { <span>{row.name.clone()}</span> }.into_any(),
                                    };
                                    view! {
                                        <tr class:bg-zinc-800=is_me class:opacity-50=withdrawn>
                                            <td class="px-3 py-2 text-zinc-500 tabular-nums">{row.rank}</td>
                                            <td class="px-3 py-2 text-white truncate" class:font-semibold=is_me>
                                                {name}
                                                {withdrawn.then(|| view! {
                                                    <span class="ml-2 eyebrow-sm text-zinc-500">"withdrawn"</span>
                                                })}
                                            </td>
                                            {started.then(|| view! {
                                                <td class="px-3 py-2 text-right text-zinc-400 tabular-nums whitespace-nowrap">
                                                    {format!("{}/{}/{}", row.wins, row.draws, row.losses)}
                                                </td>
                                            })}
                                            {show_tiebreak.then(|| view! {
                                                <td class="px-3 py-2 text-right text-zinc-500 tabular-nums">
                                                    {format_score2(row.tiebreak2)}
                                                </td>
                                            })}
                                            {started.then(|| view! {
                                                <td class="px-3 py-2 text-right font-semibold text-white tabular-nums">
                                                    {format_score2(row.score2)}
                                                </td>
                                            })}
                                            {can_manage.then(|| view! {
                                                <td class="pr-2 text-right">
                                                    <button
                                                        class="w-7 h-7 rounded-control text-zinc-500 hover:text-white hover:bg-zinc-700 transition-colors cursor-pointer"
                                                        title={if !started { "Remove" } else if withdrawn { "Bring back" } else { "Withdraw" }}
                                                        aria-label={if !started { "Remove player" } else if withdrawn { "Bring player back" } else { "Withdraw player" }}
                                                        on:click=move |_| actions.run(remove_tournament_player(id, player_id))
                                                    >
                                                        {if withdrawn && started { "↺" } else { "✕" }}
                                                    </button>
                                                </td>
                                            })}
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    </div>
                }.into_any()
            }}
        </section>
    }
}

/// The three results a game can have, as a segmented control. Clicking the
/// selected one again clears it, where clearing is allowed.
#[component]
fn ResultPicker(id: Uuid, pairing: PairingView, can_clear: bool, actions: Actions) -> impl IntoView {
    let pairing_id = pairing.id;
    let current = pairing.result;
    let option = move |result: PairingResult, label: &'static str| {
        let selected = current == Some(result);
        view! {
            <button
                type="button"
                class="px-2 py-1 text-xs font-semibold tabular-nums transition-colors cursor-pointer"
                class:bg-white=selected
                class:text-zinc-950=selected
                class:text-zinc-400=!selected
                class=("hover:text-white", !selected)
                aria-pressed=selected.to_string()
                on:click=move |_| {
                    let next = if selected && can_clear { None } else { Some(result) };
                    actions.run(set_pairing_result(id, pairing_id, next));
                }
            >
                {label}
            </button>
        }
    };
    view! {
        <div class="inline-flex items-center rounded-control bg-zinc-950 border border-zinc-800 overflow-hidden divide-x divide-zinc-800">
            {option(PairingResult::White, "1–0")}
            {option(PairingResult::Draw, "½–½")}
            {option(PairingResult::Black, "0–1")}
        </div>
    }
}

#[component]
fn PairingRowView(
    id: Uuid,
    pairing: PairingView,
    /// The organizer may set this pairing's result.
    editable: bool,
    can_clear: bool,
    my_player: Option<Uuid>,
    actions: Actions,
) -> impl IntoView {
    let mine = my_player.is_some_and(|me| pairing.white_id == me || pairing.black_id == Some(me));
    let white_won = pairing.result == Some(PairingResult::White);
    let black_won = pairing.result == Some(PairingResult::Black);

    let middle = if pairing.black_id.is_none() {
        view! { <span class="eyebrow-sm text-zinc-500">"bye"</span> }.into_any()
    } else if editable {
        view! { <ResultPicker id=id pairing=pairing.clone() can_clear=can_clear actions=actions/> }.into_any()
    } else {
        match (pairing.result, pairing.game_id) {
            (Some(result), Some(game)) if result != PairingResult::Void => view! {
                <a
                    href=format!("/analysis?game={game}")
                    class="text-sm font-semibold text-zinc-200 tabular-nums hover:underline whitespace-nowrap"
                >
                    {result.label()}
                </a>
            }.into_any(),
            (Some(result), _) => view! {
                <span class="text-sm font-semibold text-zinc-400 tabular-nums whitespace-nowrap">{result.label()}</span>
            }.into_any(),
            (None, Some(game)) => view! {
                <a
                    href=format!("/game/{game}")
                    class="eyebrow-sm text-emerald-400 hover:underline whitespace-nowrap"
                >
                    "Playing \u{2192}"
                </a>
            }.into_any(),
            (None, None) => view! { <span class="text-sm text-zinc-600">"–"</span> }.into_any(),
        }
    };

    view! {
        <div class="grid grid-cols-[1fr_auto_1fr] items-center gap-3 px-4 py-2.5" class:bg-zinc-800=mine>
            <span class="flex items-center gap-2 min-w-0 text-sm" class:text-white=white_won class:font-semibold=white_won class:text-zinc-300=!white_won>
                <span class="w-2.5 h-2.5 rounded-sm bg-zinc-100 border border-zinc-500 flex-shrink-0" title="White"></span>
                <span class="truncate">{pairing.white_name.clone()}</span>
            </span>
            {middle}
            <span class="flex items-center justify-end gap-2 min-w-0 text-sm" class:text-white=black_won class:font-semibold=black_won class:text-zinc-300=!black_won>
                <span class="truncate">{pairing.black_name.clone().unwrap_or_default()}</span>
                {pairing.black_id.is_some().then(|| view! {
                    <span class="w-2.5 h-2.5 rounded-sm bg-zinc-950 border border-zinc-500 flex-shrink-0" title="Black"></span>
                })}
            </span>
        </div>
    }
}

#[component]
fn Pairings(
    id: Uuid,
    summary: Memo<TournamentSummary>,
    pairings: Memo<Vec<PairingView>>,
    viewer: Memo<TournamentViewer>,
    actions: Actions,
) -> impl IntoView {
    move || {
        let s = summary.get();
        let v = viewer.get();
        let all = pairings.get();
        if all.is_empty() {
            return ().into_any();
        }
        let otb = s.venue == TournamentVenue::Otb;
        let live = s.status == TournamentStatus::Live;
        let organizer = v.is_organizer && live && s.format != TournamentFormat::Arena;

        let row = move |p: PairingView| {
            // Online, the organizer can only overrule a game that has
            // already reported — a forfeit for an aborted game, say.
            let editable = organizer && p.black_id.is_some() && (otb || p.game_id.is_none() || p.result.is_some());
            view! {
                <PairingRowView
                    id=id
                    pairing=p
                    editable=editable
                    can_clear=otb
                    my_player=v.player_id
                    actions=actions
                />
            }
        };

        if s.format == TournamentFormat::Arena {
            return view! {
                <section class="flex flex-col gap-3">
                    <h2 class="eyebrow text-zinc-500">"Recent games"</h2>
                    <div class="flex flex-col divide-y divide-zinc-800 surface-card overflow-hidden">
                        {all.into_iter().map(row).collect_view()}
                    </div>
                </section>
            }
            .into_any();
        }

        // Newest round first: the one in progress is the one people came for.
        let mut rounds: Vec<i32> = all.iter().map(|p| p.round).collect();
        rounds.sort_unstable();
        rounds.dedup();
        rounds.reverse();
        view! {
            <div class="flex flex-col gap-6">
                {rounds.into_iter().map(|round| {
                    let games: Vec<PairingView> = all.iter().filter(|p| p.round == round).cloned().collect();
                    view! {
                        <section class="flex flex-col gap-3">
                            <h2 class="eyebrow text-zinc-500">{format!("Round {round}")}</h2>
                            <div class="flex flex-col divide-y divide-zinc-800 surface-card overflow-hidden">
                                {games.into_iter().map(row).collect_view()}
                            </div>
                        </section>
                    }
                }).collect_view()}
            </div>
        }
        .into_any()
    }
}

/// A loaded tournament page.
///
/// Takes its first state as a prop and owns it from there. The page polls
/// rather than holding a socket open, and the polling deliberately does
/// *not* go through the `Resource` that loaded `initial`: refetching that
/// would rebuild this whole subtree every few seconds, taking the
/// organizer's half-typed player name and every button's in-flight state
/// with it. Instead the fresh view lands in one signal and each section
/// reads its own slice through a `Memo`, so a poll that changed nothing
/// re-renders nothing.
#[component]
pub fn TournamentBody(initial: TournamentView) -> impl IntoView {
    let id = initial.summary.id;
    // Server-clock seconds. Seeded from the server and advanced locally, so
    // countdowns don't depend on the browser's clock being right; every
    // poll re-seeds it.
    let now = RwSignal::new(initial.server_now);
    let state = RwSignal::new(initial);

    let summary = Memo::new(move |_| state.with(|v| v.summary.clone()));
    let viewer = Memo::new(move |_| state.with(|v| v.viewer.clone()));
    let standings = Memo::new(move |_| state.with(|v| v.standings.clone()));
    let pairings = Memo::new(move |_| state.with(|v| v.pairings.clone()));

    let refresh = Callback::new(move |_: ()| {
        #[cfg(feature = "hydrate")]
        leptos::task::spawn_local(async move {
            if let Ok(view) = crate::tournament::get_tournament(id).await {
                now.set(view.server_now);
                state.set(view);
            }
        });
    });
    let actions = Actions { error: RwSignal::new(None), busy: RwSignal::new(false), refresh };

    #[cfg(feature = "hydrate")]
    {
        use leptos_use::use_interval_fn;

        let since_poll = StoredValue::new(0u32);
        use_interval_fn(
            move || {
                now.update(|n| *n += 1);
                let due = poll_seconds(&summary.get_untracked(), now.get_untracked());
                let waited = since_poll.get_value() + 1;
                match due {
                    Some(interval) if waited >= interval => {
                        since_poll.set_value(0);
                        refresh.run(());
                    }
                    _ => since_poll.set_value(waited),
                }
            },
            1_000,
        );

        // Paired: go and play. This is the whole "you have a game" signal —
        // the server reports the viewer's in-progress tournament game with
        // every poll, so it works the same whichever instance created it,
        // and a player who wanders back here mid-game is sent straight back
        // to their board.
        let navigate = leptos_router::hooks::use_navigate();
        Effect::new(move |_| {
            if let Some(game) = viewer.get().active_game {
                navigate(&format!("/game/{game}"), Default::default());
            }
        });
    }

    // The first three finishers, once it is over. Only people who actually
    // played: in a tournament nobody turned up to, there is no podium.
    let podium = move || {
        if summary.get().status != TournamentStatus::Finished {
            return Vec::new();
        }
        standings.get().into_iter().filter(|r| r.played > 0 || r.score2 > 0).take(3).collect::<Vec<_>>()
    };
    let can_add_players = move || {
        let s = summary.get();
        viewer.get().is_organizer
            && s.venue == TournamentVenue::Otb
            && match s.status {
                TournamentStatus::Upcoming => true,
                // A swiss takes late entries; a round robin's schedule is fixed.
                TournamentStatus::Live => s.format == TournamentFormat::Swiss,
                _ => false,
            }
    };

    view! {
        <Title text=move || summary.get().name/>
        <div class="px-6 py-8 max-w-5xl mx-auto flex flex-col gap-6">
            <header class="flex flex-col gap-3">
                <a href="/tournaments" class="text-xs text-zinc-500 hover:text-zinc-300 transition-colors self-start">
                    "\u{2190} All tournaments"
                </a>
                <div class="flex flex-wrap items-end justify-between gap-x-6 gap-y-2">
                    <h1 class="display-1 text-white min-w-0 break-words">{move || summary.get().name}</h1>
                    <span
                        class="text-lg font-semibold tabular-nums whitespace-nowrap"
                        class:text-emerald-400=move || summary.get().status == TournamentStatus::Live
                        class:text-zinc-300=move || summary.get().status != TournamentStatus::Live
                    >
                        {move || status_line(&summary.get(), now.get())}
                    </span>
                </div>
                <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
                    {move || view! { <TournamentTags summary=summary.get()/> }}
                    <span class="text-xs text-zinc-500">
                        {move || {
                            let s = summary.get();
                            let players = format!(
                                "{} {}",
                                s.player_count,
                                if s.player_count == 1 { "player" } else { "players" },
                            );
                            match s.organizer {
                                Some(name) => format!("{players} · organized by {name}"),
                                None => players,
                            }
                        }}
                    </span>
                </div>
            </header>

            {move || {
                let top = podium();
                (!top.is_empty()).then(|| view! {
                    <div class="grid grid-cols-1 sm:grid-cols-3 gap-3">
                        {top.into_iter().enumerate().map(|(i, row)| {
                            let (medal, label) = [("\u{1F947}", "Winner"), ("\u{1F948}", "Second"), ("\u{1F949}", "Third")][i];
                            view! {
                                <div class="surface-card px-5 py-4 flex items-center gap-4" class:accent-ring=i == 0>
                                    <span class="text-3xl leading-none flex-shrink-0" aria-hidden="true">{medal}</span>
                                    <div class="flex flex-col gap-0.5 min-w-0 flex-1">
                                        <span class="eyebrow-sm" class:accent-text=i == 0 class:text-zinc-500=i != 0>{label}</span>
                                        <span class="text-lg font-bold text-white truncate">{row.name.clone()}</span>
                                    </div>
                                    <span class="text-xl font-bold text-white tabular-nums flex-shrink-0">
                                        {format_score2(row.score2)}
                                    </span>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                })
            }}

            <div class="flex flex-col gap-3">
                <EntryControls id=id summary=summary viewer=viewer actions=actions/>
                <Show when=move || {
                    let (s, v) = (summary.get(), viewer.get());
                    s.venue == TournamentVenue::Online && !s.status.is_over() && v.can_enter
                }>
                    <InvitePanel id=id actions=actions/>
                </Show>
                <Show when=move || viewer.get().is_organizer>
                    <OrganizerControls id=id summary=summary pairings=pairings standings=standings actions=actions/>
                </Show>
                {move || actions.error.get().map(|message| view! {
                    <p class="text-sm text-red-400" role="alert">{message}</p>
                })}
            </div>

            <div class="grid grid-cols-1 lg:grid-cols-2 gap-8 items-start">
                <div class="flex flex-col gap-4">
                    <Standings id=id summary=summary standings=standings viewer=viewer actions=actions/>
                    <Show when=can_add_players>
                        <AddPlayerForm id=id actions=actions/>
                    </Show>
                </div>
                <Pairings id=id summary=summary pairings=pairings viewer=viewer actions=actions/>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(status: TournamentStatus, format: TournamentFormat, venue: TournamentVenue) -> TournamentSummary {
        TournamentSummary {
            id: Uuid::nil(),
            name: "T".into(),
            format,
            venue,
            status,
            initial_seconds: 180,
            increment_seconds: 0,
            rated: false,
            starts_at: 10_000,
            duration_minutes: Some(30),
            total_rounds: None,
            current_round: 0,
            player_count: 0,
            organizer: None,
            official: false,
            mine: false,
        }
    }

    #[test]
    fn a_live_arena_polls_well_inside_the_lobby_window() {
        use TournamentFormat::*;
        use TournamentStatus::*;
        use TournamentVenue::*;
        // The server forgets an entrant after 12s of silence; three polls
        // must fit in that.
        let poll = poll_seconds(&summary(Live, Arena, Online), 10_100).unwrap();
        assert!(poll * 3 <= 12);
    }

    #[test]
    fn nothing_polls_once_a_tournament_is_over() {
        use TournamentFormat::*;
        use TournamentStatus::*;
        use TournamentVenue::*;
        assert_eq!(poll_seconds(&summary(Finished, Arena, Online), 0), None);
        assert_eq!(poll_seconds(&summary(Cancelled, Swiss, Otb), 0), None);
    }

    #[test]
    fn an_upcoming_arena_polls_faster_as_its_start_approaches() {
        use TournamentFormat::*;
        use TournamentStatus::*;
        use TournamentVenue::*;
        let s = summary(Upcoming, Arena, Online);
        let far = poll_seconds(&s, s.starts_at - 3_600).unwrap();
        let near = poll_seconds(&s, s.starts_at - 30).unwrap();
        assert!(near < far);
    }
}
