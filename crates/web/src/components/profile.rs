use leptos::prelude::*;
use shared::{Category, FriendRelation, ProfileView};
use strum::IntoEnumIterator;

use crate::components::{ChallengeModal, EloCard, FriendSearch, FriendsList, RecentGames};
use crate::friends::{
    get_profile, use_friends_presence, CancelFriendRequest, RespondFriendRequest, SendFriendRequest,
    Unfriend,
};

fn display_name(profile: &ProfileView) -> String {
    profile.user.username.clone().unwrap_or_else(|| "Anonymous".to_string())
}

/// The header actions available when viewing *someone else's* profile —
/// mirrors the relation-driven button set already used per-row in
/// `friend_search.rs`/`friends_list.rs`, just scoped to one person instead
/// of a list.
#[component]
fn ProfileActions(profile: ProfileView) -> impl IntoView {
    let presence = use_friends_presence();
    let challenging = RwSignal::new(false);

    let send_action = ServerAction::<SendFriendRequest>::new();
    let respond_action = ServerAction::<RespondFriendRequest>::new();
    let cancel_action = ServerAction::<CancelFriendRequest>::new();
    let remove_action = ServerAction::<Unfriend>::new();

    // None of these pushes reach the viewer's own client (they're all
    // addressed to the *other* party) — bump list_trigger locally so the
    // profile's own `get_profile` resource (keyed on it) refetches, same
    // fix as friends_list.rs/friend_search.rs.
    Effect::new(move |_| {
        if send_action.value().get().is_some_and(|r| r.is_ok())
            || respond_action.value().get().is_some_and(|r| r.is_ok())
            || cancel_action.value().get().is_some_and(|r| r.is_ok())
            || remove_action.value().get().is_some_and(|r| r.is_ok())
        {
            presence.list_trigger.update(|v| *v += 1);
        }
    });

    let target_id = profile.user.id;
    let online = profile.online;
    let in_game = profile.in_game.clone();
    let relation = profile.relation;

    view! {
        <div class="flex items-center gap-2">
            {move || match relation {
                FriendRelation::None => view! {
                    <button
                        type="button"
                        on:click=move |_| { send_action.dispatch(SendFriendRequest { target_id }); }
                        class="px-4 py-2 text-sm font-semibold bg-white text-zinc-950 rounded-md hover:bg-zinc-100 transition-colors cursor-pointer"
                    >
                        "Add Friend"
                    </button>
                }.into_any(),
                FriendRelation::PendingOutgoing => view! {
                    <button
                        type="button"
                        on:click=move |_| { cancel_action.dispatch(CancelFriendRequest { target_id }); }
                        class="px-4 py-2 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                    >
                        "Cancel request"
                    </button>
                }.into_any(),
                FriendRelation::PendingIncoming => view! {
                    <div class="flex gap-2">
                        <button
                            type="button"
                            on:click=move |_| { respond_action.dispatch(RespondFriendRequest { requester_id: target_id, accept: true }); }
                            class="px-4 py-2 text-sm font-semibold bg-emerald-600 text-white rounded-md hover:bg-emerald-500 transition-colors cursor-pointer"
                        >
                            "Accept"
                        </button>
                        <button
                            type="button"
                            on:click=move |_| { respond_action.dispatch(RespondFriendRequest { requester_id: target_id, accept: false }); }
                            class="px-4 py-2 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                        >
                            "Decline"
                        </button>
                    </div>
                }.into_any(),
                FriendRelation::Friends => {
                    let in_game = in_game.clone();
                    view! {
                        <div class="flex gap-2">
                            {match in_game {
                                Some(g) => view! {
                                    <a
                                        href={format!("/game/{}", g.game_id)}
                                        class="px-4 py-2 text-sm font-semibold text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-white transition-colors"
                                    >
                                        "Watch"
                                    </a>
                                }.into_any(),
                                None => view! {
                                    <button
                                        type="button"
                                        disabled=!online
                                        on:click=move |_| challenging.set(true)
                                        class="px-4 py-2 text-sm font-semibold rounded-md transition-colors"
                                        class:bg-white=online
                                        class:text-zinc-950=online
                                        class:cursor-pointer=online
                                        class:hover:bg-zinc-100=online
                                        class:bg-zinc-800=!online
                                        class:text-zinc-600=!online
                                        class:cursor-not-allowed=!online
                                    >
                                        "Challenge"
                                    </button>
                                }.into_any(),
                            }}
                            <button
                                type="button"
                                on:click=move |_| { remove_action.dispatch(Unfriend { other_id: target_id }); }
                                class="px-4 py-2 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-md hover:border-zinc-500 hover:text-red-400 transition-colors cursor-pointer"
                            >
                                "Remove friend"
                            </button>
                        </div>
                    }.into_any()
                }
            }}
        </div>
        <Show when=move || challenging.get()>
            <ChallengeModal friend=profile.user.clone() on_close=Callback::new(move |_| challenging.set(false)) />
        </Show>
    }
}

#[component]
pub fn Profile(username: String) -> impl IntoView {
    let presence = use_friends_presence();

    let username_for_resource = username.clone();
    let profile = Resource::new(
        move || (username_for_resource.clone(), presence.list_trigger.get()),
        |(username, _)| get_profile(username),
    );

    let fallback = move || {
        view! {
            <div class="flex flex-col gap-8">
                <div class="h-20 rounded-2xl skeleton-shimmer"/>
                <div class="h-32 rounded-2xl skeleton-shimmer"/>
            </div>
        }
    };

    // EloCard/RecentGames below are unconditional siblings — constructed
    // regardless of `profile`'s state, keyed only on the `username` prop
    // (available synchronously), never nested *inside* the `Ok` branch below
    // (which only exists once `profile` has already resolved). A fresh
    // `Resource` conditionally constructed only after another resource
    // resolves intermittently desynced SSR and hydration for these sibling
    // resources (a hydration panic in `RecentGameRow`, roughly 1 in 3 fresh
    // loads). They do still sit inside this *same* outer `<Transition>` as
    // the profile-dependent header/friends sections, though — nested
    // Suspense/Transition tracking bubbles up, so the one shared fallback
    // below covers all of them together. Splitting them into separate
    // `<Transition>` boundaries (an earlier version of this page) also
    // avoided the hydration bug, but caused a staggered pop-in on
    // client-side navigation (header, then each rating card, then games,
    // then friends, each resolving at a slightly different moment) that
    // read as a flicker.
    view! {
        <div class="px-6 py-8 max-w-2xl mx-auto flex flex-col gap-8">
            <Transition fallback=fallback>
                <div class="flex flex-col gap-8">
                    {move || profile.get().map(|result| match result {
                        Err(e) => view! {
                            <p class="text-zinc-500">"Couldn't load this profile: " {e.to_string()}</p>
                        }.into_any(),
                        Ok(profile) => {
                            let is_own = profile.is_own;
                            let name = display_name(&profile);
                            let avatar_url = profile.user.avatar_url.clone();
                            let bio = profile.bio.clone();
                            let online = profile.online;

                            view! {
                                <div class="flex items-start justify-between gap-4 flex-wrap">
                                    <div class="flex items-center gap-4 min-w-0">
                                        {avatar_url.map(|url| view! {
                                            <img src={url} class="w-16 h-16 rounded-full flex-shrink-0" />
                                        })}
                                        <div class="flex flex-col gap-1 min-w-0">
                                            <div class="flex items-center gap-2">
                                                <h1 class="text-2xl font-bold tracking-tighter text-white truncate">{name}</h1>
                                                <Show when=move || !is_own>
                                                    <span
                                                        class="w-2 h-2 rounded-full flex-shrink-0"
                                                        class:bg-emerald-500=online
                                                        class:bg-zinc-700=!online
                                                    />
                                                </Show>
                                            </div>
                                            {bio.map(|b| view! { <p class="text-sm text-zinc-500">{b}</p> })}
                                        </div>
                                    </div>
                                    <Show when=move || !is_own>
                                        <ProfileActions profile=profile.clone()/>
                                    </Show>
                                </div>
                            }.into_any()
                        }
                    })}

                    <div class="flex gap-3 overflow-x-auto scrollbar-none">
                        {Category::iter().map(|c| {
                            let username = username.clone();
                            view! { <EloCard category={c} username={username}/> }
                        }).collect_view()}
                    </div>
                    <RecentGames username={username.clone()}/>

                    {move || profile.get().and_then(|r| r.ok()).map(|profile| {
                        let is_own = profile.is_own;
                        view! {
                            <div class="flex flex-col gap-4">
                                <Show when=move || is_own>
                                    <FriendSearch/>
                                </Show>
                                <FriendsList
                                    friends=profile.friends.clone()
                                    is_own=is_own
                                    incoming=profile.incoming_requests.clone()
                                    outgoing=profile.outgoing_requests.clone()
                                />
                            </div>
                        }
                    })}
                </div>
            </Transition>
        </div>
    }
}
