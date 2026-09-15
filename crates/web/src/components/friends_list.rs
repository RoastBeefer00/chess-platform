use leptos::prelude::*;
use shared::{FriendRow, FriendSummary, RatingMode, TimeControl, TimeMode};

use crate::friends::{
    use_friends_presence, CancelFriendRequest, OutgoingChallenge, RespondFriendRequest, SendChallenge,
    Unfriend,
};

fn display_name(u: &FriendSummary) -> String {
    u.username.clone().unwrap_or_else(|| "Anonymous".to_string())
}

fn profile_href(u: &FriendSummary) -> Option<String> {
    u.username.as_ref().map(|name| format!("/u/{name}"))
}

const CHALLENGE_TIME_CONTROLS: &[(&str, i64, i64)] =
    &[("1+0", 60_000, 0), ("3+0", 180_000, 0), ("5+0", 300_000, 0), ("10+0", 600_000, 0)];

/// Also reused directly by the profile page's header "Challenge" action
/// (viewing a friend's profile), not just from within the friends list.
#[component]
pub fn ChallengeModal(friend: FriendSummary, on_close: Callback<()>) -> impl IntoView {
    let presence = use_friends_presence();
    let rating_mode = RwSignal::new(RatingMode::Rated);
    let challenge_action = ServerAction::<SendChallenge>::new();

    let friend_for_effect = friend.clone();
    Effect::new(move |_| {
        if let Some(Ok(challenge_id)) = challenge_action.value().get() {
            presence.outgoing.set(Some(OutgoingChallenge {
                challenge_id,
                to_username: display_name(&friend_for_effect),
            }));
            on_close.run(());
        }
    });

    let friend_id = friend.id;
    view! {
        <div
            class="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
            on:click=move |_| on_close.run(())
        >
            <div
                class="flex flex-col gap-4 rounded-2xl bg-zinc-900 border border-zinc-800 p-6 w-full max-w-xs mx-4"
                on:click=move |ev| ev.stop_propagation()
            >
                <h2 class="text-lg font-bold text-white">
                    "Challenge " {display_name(&friend)}
                </h2>

                <div class="flex items-center rounded-lg bg-zinc-950 border border-zinc-800/60 p-0.5 self-start">
                    <button
                        type="button"
                        on:click=move |_| rating_mode.set(RatingMode::Rated)
                        class="px-3 py-1.5 text-xs font-semibold uppercase tracking-wide rounded-md transition-colors cursor-pointer"
                        class:bg-white=move || rating_mode.get() == RatingMode::Rated
                        class:text-zinc-950=move || rating_mode.get() == RatingMode::Rated
                        class:text-zinc-400=move || rating_mode.get() != RatingMode::Rated
                    >
                        "Rated"
                    </button>
                    <button
                        type="button"
                        on:click=move |_| rating_mode.set(RatingMode::Casual)
                        class="px-3 py-1.5 text-xs font-semibold uppercase tracking-wide rounded-md transition-colors cursor-pointer"
                        class:bg-white=move || rating_mode.get() == RatingMode::Casual
                        class:text-zinc-950=move || rating_mode.get() == RatingMode::Casual
                        class:text-zinc-400=move || rating_mode.get() != RatingMode::Casual
                    >
                        "Casual"
                    </button>
                </div>

                <div class="grid grid-cols-2 gap-2">
                    {CHALLENGE_TIME_CONTROLS.iter().map(|&(label, ms, inc)| {
                        view! {
                            <button
                                type="button"
                                on:click=move |_| {
                                    challenge_action.dispatch(SendChallenge {
                                        target_id: friend_id,
                                        time_control: TimeControl { initial_time: ms, mode: TimeMode::Increment(inc) },
                                        rating_mode: rating_mode.get(),
                                    });
                                }
                                class="px-4 py-3 rounded-xl bg-zinc-800 border border-zinc-700 hover:bg-zinc-700 transition-colors cursor-pointer text-white font-semibold"
                            >
                                {label}
                            </button>
                        }
                    }).collect_view()}
                </div>

                {move || challenge_action.value().get().and_then(|r| r.err()).map(|e| view! {
                    <p class="text-xs text-red-400">{e.to_string()}</p>
                })}

                <button
                    type="button"
                    on:click=move |_| on_close.run(())
                    class="text-xs text-zinc-500 hover:text-zinc-300 transition-colors cursor-pointer self-center"
                >
                    "Cancel"
                </button>
            </div>
        </div>
    }
}

#[component]
fn FriendRowView(row: FriendRow, is_own: bool) -> impl IntoView {
    let presence = use_friends_presence();
    let challenging = RwSignal::new(false);
    let remove_action = ServerAction::<Unfriend>::new();

    // `unfriend` only pushes `FriendListChanged` to the *other* party over
    // their presence socket — our own client has no such push coming back,
    // so refresh our own list locally on success.
    Effect::new(move |_| {
        if remove_action.value().get().is_some_and(|r| r.is_ok()) {
            presence.list_trigger.update(|v| *v += 1);
        }
    });

    let friend = row.user.clone();
    let friend_id = friend.id;
    let href = profile_href(&friend);

    let online = Signal::derive(move || {
        presence.online.get().get(&friend_id).copied().unwrap_or(row.online)
    });
    let row_for_in_game = row.clone();
    let in_game_id = Signal::derive(move || {
        match presence.in_game.get().get(&friend_id) {
            Some(over) => *over,
            None => row_for_in_game.in_game.as_ref().map(|g| g.game_id),
        }
    });

    view! {
        <div class="flex items-center justify-between gap-3 px-4 py-3">
            <div class="flex items-center gap-3 min-w-0">
                <span
                    class="w-2 h-2 rounded-full flex-shrink-0"
                    class:bg-emerald-500=online
                    class:bg-zinc-700=move || !online.get()
                />
                {friend.avatar_url.clone().map(|url| view! {
                    <img src={url} class="w-8 h-8 rounded-full flex-shrink-0" />
                })}
                {match href.clone() {
                    Some(href) => view! {
                        <a href={href} class="text-sm font-medium text-white truncate hover:underline">
                            {display_name(&friend)}
                        </a>
                    }.into_any(),
                    None => view! {
                        <span class="text-sm font-medium text-white truncate">{display_name(&friend)}</span>
                    }.into_any(),
                }}
            </div>
            {is_own.then(|| view! {
                <div class="flex items-center gap-2 flex-shrink-0">
                    {move || match in_game_id.get() {
                        Some(game_id) => view! {
                            <a
                                href={format!("/game/{game_id}")}
                                class="px-3 py-1.5 text-xs font-semibold text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors"
                            >
                                "Watch"
                            </a>
                        }.into_any(),
                        None => view! {
                            <button
                                type="button"
                                disabled=move || !online.get()
                                on:click=move |_| challenging.set(true)
                                class="px-3 py-1.5 text-xs font-semibold rounded-md transition-colors"
                                class:bg-white=online
                                class:text-zinc-950=online
                                class:cursor-pointer=online
                                class:hover:bg-zinc-100=online
                                class:bg-zinc-800=move || !online.get()
                                class:text-zinc-600=move || !online.get()
                                class:cursor-not-allowed=move || !online.get()
                            >
                                "Challenge"
                            </button>
                        }.into_any(),
                    }}
                    <button
                        type="button"
                        on:click=move |_| { remove_action.dispatch(Unfriend { other_id: friend_id }); }
                        title="Remove friend"
                        aria-label="Remove friend"
                        class="w-7 h-7 flex items-center justify-center text-zinc-600 hover:text-red-400 transition-colors cursor-pointer"
                    >
                        "\u{2715}"
                    </button>
                </div>
            })}
        </div>
        <Show when=move || challenging.get()>
            <ChallengeModal friend=friend.clone() on_close=Callback::new(move |_| challenging.set(false)) />
        </Show>
    }
}

#[component]
fn PendingRequestRow(user: FriendSummary) -> impl IntoView {
    let presence = use_friends_presence();
    let respond_action = ServerAction::<RespondFriendRequest>::new();
    let requester_id = user.id;

    // Same reason as `FriendRowView`'s remove_action effect: the push only
    // reaches the requester, not us.
    Effect::new(move |_| {
        if respond_action.value().get().is_some_and(|r| r.is_ok()) {
            presence.list_trigger.update(|v| *v += 1);
        }
    });

    view! {
        <div class="flex items-center justify-between gap-3 px-4 py-3">
            <span class="text-sm font-medium text-white truncate">{display_name(&user)}</span>
            <div class="flex gap-2 flex-shrink-0">
                <button
                    type="button"
                    on:click=move |_| { respond_action.dispatch(RespondFriendRequest { requester_id, accept: true }); }
                    class="px-3 py-1.5 text-xs font-semibold bg-emerald-600 text-white rounded-md hover:bg-emerald-500 transition-colors cursor-pointer"
                >
                    "Accept"
                </button>
                <button
                    type="button"
                    on:click=move |_| { respond_action.dispatch(RespondFriendRequest { requester_id, accept: false }); }
                    class="px-3 py-1.5 text-xs font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                >
                    "Decline"
                </button>
            </div>
        </div>
    }
}

#[component]
fn OutgoingRequestRow(user: FriendSummary) -> impl IntoView {
    let presence = use_friends_presence();
    let cancel_action = ServerAction::<CancelFriendRequest>::new();
    let target_id = user.id;

    Effect::new(move |_| {
        if cancel_action.value().get().is_some_and(|r| r.is_ok()) {
            presence.list_trigger.update(|v| *v += 1);
        }
    });

    view! {
        <div class="flex items-center justify-between gap-3 px-4 py-3">
            <span class="text-sm text-zinc-400 truncate">{display_name(&user)}</span>
            <div class="flex items-center gap-2 flex-shrink-0">
                <span class="text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-600">"Pending"</span>
                <button
                    type="button"
                    on:click=move |_| { cancel_action.dispatch(CancelFriendRequest { target_id }); }
                    class="px-2 py-1 text-[10px] font-medium text-zinc-500 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                >
                    "Cancel"
                </button>
            </div>
        </div>
    }
}

/// Presentational friends section for a profile page — the caller (the
/// profile page) already fetched everything via `get_profile`, so this
/// component just renders it. `incoming`/`outgoing` are only ever non-empty
/// when `is_own` (the server only populates them for the profile's owner),
/// but the split is enforced by `get_profile`, not re-checked here.
#[component]
pub fn FriendsList(
    friends: Vec<FriendRow>,
    is_own: bool,
    incoming: Vec<FriendSummary>,
    outgoing: Vec<FriendSummary>,
) -> impl IntoView {
    let has_incoming = !incoming.is_empty();
    let has_outgoing = !outgoing.is_empty();

    view! {
        <div class="flex flex-col gap-6">
            <Show when=move || has_incoming>
                <div class="flex flex-col gap-2">
                    <h2 class="text-sm font-semibold uppercase tracking-wide text-zinc-500 px-1">
                        "Friend requests"
                    </h2>
                    <div class="flex flex-col divide-y divide-zinc-800 rounded-2xl bg-zinc-900 border border-zinc-800 overflow-hidden">
                        {incoming.iter().cloned().map(|u| view! { <PendingRequestRow user=u/> }).collect_view()}
                    </div>
                </div>
            </Show>

            <Show when=move || has_outgoing>
                <div class="flex flex-col gap-2">
                    <h2 class="text-sm font-semibold uppercase tracking-wide text-zinc-500 px-1">
                        "Sent requests"
                    </h2>
                    <div class="flex flex-col divide-y divide-zinc-800 rounded-2xl bg-zinc-900 border border-zinc-800 overflow-hidden">
                        {outgoing.iter().cloned().map(|u| view! { <OutgoingRequestRow user=u/> }).collect_view()}
                    </div>
                </div>
            </Show>

            <div class="flex flex-col gap-2">
                <h2 class="text-sm font-semibold uppercase tracking-wide text-zinc-500 px-1">
                    "Friends"
                </h2>
                {if friends.is_empty() {
                    let empty_label = if is_own { "No friends yet — search above to add some" } else { "No friends yet" };
                    view! { <p class="text-zinc-500 text-sm italic px-1">{empty_label}</p> }.into_any()
                } else {
                    view! {
                        <div class="flex flex-col divide-y divide-zinc-800 rounded-2xl bg-zinc-900 border border-zinc-800 overflow-hidden">
                            {friends.iter().cloned().map(|row| view! { <FriendRowView row=row is_own=is_own/> }).collect_view()}
                        </div>
                    }.into_any()
                }}
            </div>
        </div>
    }
}
