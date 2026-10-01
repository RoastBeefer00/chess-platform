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

    let spectate_href = in_game.as_ref().map(|g| format!("/game/{}", g.game_id));

    view! {
        <div class="flex items-center gap-2">
            // Any game in progress is watchable by anyone — this used to
            // appear only for friends, so a stranger's profile gave no way
            // in even though `/watch` would happily show the same game.
            {spectate_href.map(|href| view! {
                <a
                    href={href}
                    class="px-4 py-2 text-sm font-semibold text-zinc-300 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-white transition-colors"
                >
                    "Watch game"
                </a>
            })}
            {move || match relation {
                FriendRelation::None => view! {
                    <button
                        type="button"
                        on:click=move |_| { send_action.dispatch(SendFriendRequest { target_id }); }
                        class="px-4 py-2 text-sm font-semibold bg-white text-zinc-950 rounded-control hover:bg-zinc-100 transition-colors cursor-pointer"
                    >
                        "Add Friend"
                    </button>
                }.into_any(),
                FriendRelation::PendingOutgoing => view! {
                    <button
                        type="button"
                        on:click=move |_| { cancel_action.dispatch(CancelFriendRequest { target_id }); }
                        class="px-4 py-2 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                    >
                        "Cancel request"
                    </button>
                }.into_any(),
                FriendRelation::PendingIncoming => view! {
                    <div class="flex gap-2">
                        <button
                            type="button"
                            on:click=move |_| { respond_action.dispatch(RespondFriendRequest { requester_id: target_id, accept: true }); }
                            class="px-4 py-2 text-sm font-semibold bg-emerald-600 text-white rounded-control hover:bg-emerald-500 transition-colors cursor-pointer"
                        >
                            "Accept"
                        </button>
                        <button
                            type="button"
                            on:click=move |_| { respond_action.dispatch(RespondFriendRequest { requester_id: target_id, accept: false }); }
                            class="px-4 py-2 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                        >
                            "Decline"
                        </button>
                    </div>
                }.into_any(),
                FriendRelation::Friends => {
                    let is_in_game = in_game.is_some();
                    view! {
                        <div class="flex gap-2">
                            {match is_in_game {
                                // The "Watch game" link above already covers
                                // this case; a player mid-game can't also be
                                // challenged.
                                true => ().into_any(),
                                false => view! {
                                    <button
                                        type="button"
                                        disabled=!online
                                        on:click=move |_| challenging.set(true)
                                        class="px-4 py-2 text-sm font-semibold rounded-control transition-colors"
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
                                class="px-4 py-2 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-red-400 transition-colors cursor-pointer"
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
                <div class="h-20 rounded-card skeleton-shimmer"/>
                <div class="h-32 rounded-card skeleton-shimmer"/>
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
        // Wider than the old `max-w-2xl`: the page is now two columns on
        // desktop rather than one stack of four equally-weighted sections.
        <div class="px-6 py-8 max-w-5xl mx-auto">
            <Transition fallback=fallback>
                <div class="flex flex-col gap-8">
                    // ── Identity band ────────────────────────────────────
                    // Given real weight (card surface, display-size name,
                    // avatar at 20 rather than 16) so the page has an
                    // obvious subject instead of four sections that all
                    // look equally important.
                    {move || profile.get().map(|result| match result {
                        Err(e) => view! {
                            <p class="text-zinc-500">"Couldn't load this profile: " {e.to_string()}</p>
                        }.into_any(),
                        Ok(profile) => {
                            let is_own = profile.is_own;
                            let name = display_name(&profile);
                            let avatar_url = profile.user.avatar_url.clone();
                            let bio = profile.bio.clone();
                            let country = profile.country.clone();
                            let online = profile.online;

                            view! {
                                <div class="surface-card p-6 flex items-start justify-between gap-6 flex-wrap">
                                    <div class="flex items-center gap-5 min-w-0">
                                        {avatar_url.map(|url| view! {
                                            <img src={url} alt="" class="w-20 h-20 rounded-full flex-shrink-0" />
                                        })}
                                        <div class="flex flex-col gap-2 min-w-0">
                                            <div class="flex items-center gap-3 min-w-0">
                                                <h1 class="display-1 text-white truncate">{name}</h1>
                                                {country.map(|cc| view! {
                                                    <img
                                                        src=format!("/flags/{}.png", cc.to_uppercase())
                                                        alt=cc.to_uppercase()
                                                        title=cc.to_uppercase()
                                                        class="w-6 h-auto rounded-sm flex-shrink-0"
                                                    />
                                                })}
                                            </div>
                                            // Status as a labelled pill rather than a bare
                                            // dot — a 8px dot with no text was the only
                                            // signal, and unreadable on its own.
                                            <Show when=move || !is_own>
                                                <span class="flex items-center gap-1.5 text-xs font-medium">
                                                    <span
                                                        class="w-2 h-2 rounded-full flex-shrink-0"
                                                        class:bg-emerald-500=online
                                                        class:bg-zinc-600=!online
                                                    />
                                                    <span class=if online { "text-emerald-400" } else { "text-zinc-500" }>
                                                        {if online { "Online" } else { "Offline" }}
                                                    </span>
                                                </span>
                                            </Show>
                                            {bio.map(|b| view! {
                                                <p class="text-sm text-zinc-400 max-w-md">{b}</p>
                                            })}
                                        </div>
                                    </div>
                                    <Show when=move || !is_own>
                                        <ProfileActions profile=profile.clone()/>
                                    </Show>
                                </div>
                            }.into_any()
                        }
                    })}

                    // ── Ratings ──────────────────────────────────────────
                    <div class="flex flex-col gap-3">
                        <h2 class="eyebrow text-zinc-500">
                            "Ratings"
                        </h2>
                        <div class="flex gap-3 overflow-x-auto scrollbar-none">
                            {Category::iter().map(|c| {
                                let username = username.clone();
                                view! { <EloCard category={c} username={username}/> }
                            }).collect_view()}
                        </div>
                    </div>

                    // ── Games and friends, side by side on desktop ───────
                    // `RecentGames` stays an unconditional sibling here, NOT
                    // nested inside the resolved branch above — see the note
                    // on the fallback for why that distinction matters.
                    <div class="grid grid-cols-1 lg:grid-cols-[1.35fr_1fr] gap-8 items-start">
                        <div class="min-w-0">
                            <RecentGames username={username.clone()}/>
                        </div>

                        <div class="min-w-0">
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
                    </div>
                </div>
            </Transition>
        </div>
    }
}
