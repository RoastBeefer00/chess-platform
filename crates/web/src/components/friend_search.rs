use leptos::prelude::*;
use shared::{FriendRelation, UserSearchResult};

use crate::friends::{search_users, use_friends_presence, SendFriendRequest};

/// Debounce delay before a keystroke actually fires a search request. The
/// `/api/*` rate limiter (30 burst, 2/sec refill) would otherwise get hit on
/// every keystroke from a fast typist.
#[cfg(feature = "hydrate")]
const DEBOUNCE_MS: u32 = 250;

fn relation_label(relation: FriendRelation) -> &'static str {
    match relation {
        FriendRelation::None => "Add",
        FriendRelation::PendingOutgoing => "Pending",
        FriendRelation::PendingIncoming => "Respond below",
        FriendRelation::Friends => "Friends",
    }
}

#[component]
fn SearchResultRow(result: UserSearchResult) -> impl IntoView {
    let presence = use_friends_presence();
    let send_action = ServerAction::<SendFriendRequest>::new();
    let target_id = result.user.id;
    let name = result.user.username.clone().unwrap_or_else(|| "Anonymous".to_string());

    let sent = RwSignal::new(false);
    Effect::new(move |_| {
        if send_action.value().get().is_some_and(|r| r.is_ok()) {
            sent.set(true);
            // `send_friend_request` only pushes `FriendListChanged` to the
            // *target* over their presence socket — refresh our own
            // "Sent requests" section locally, same as the other mutating
            // actions in `friends_list.rs`.
            presence.list_trigger.update(|v| *v += 1);
        }
    });

    let relation = result.relation;
    let actionable = move || matches!(relation, FriendRelation::None) && !sent.get();
    let href = result.user.username.as_ref().map(|u| format!("/u/{u}"));

    view! {
        <div class="flex items-center justify-between gap-3 px-4 py-3">
            <div class="flex items-center gap-3 min-w-0">
                {result.user.avatar_url.clone().map(|url| view! {
                    <img src={url} class="w-8 h-8 rounded-full flex-shrink-0" />
                })}
                {match href {
                    Some(href) => view! {
                        <a href={href} class="text-sm font-medium text-white truncate hover:underline">{name}</a>
                    }.into_any(),
                    None => view! { <span class="text-sm font-medium text-white truncate">{name}</span> }.into_any(),
                }}
            </div>
            {move || if actionable() {
                view! {
                    <button
                        type="button"
                        on:click=move |_| { send_action.dispatch(SendFriendRequest { target_id }); }
                        class="px-3 py-1.5 text-xs font-semibold bg-white text-zinc-950 rounded-md hover:bg-zinc-100 transition-colors cursor-pointer flex-shrink-0"
                    >
                        "Add"
                    </button>
                }.into_any()
            } else {
                view! {
                    <span class="text-xs text-zinc-500 flex-shrink-0">
                        {move || if sent.get() { "Pending" } else { relation_label(relation) }}
                    </span>
                }.into_any()
            }}
        </div>
    }
}

#[component]
pub fn FriendSearch() -> impl IntoView {
    let query = RwSignal::new(String::new());
    let debounced_query = RwSignal::new(String::new());
    let generation = RwSignal::new(0u64);

    let on_input = move |ev: leptos::ev::Event| {
        let value = event_target_value(&ev);
        query.set(value.clone());
        generation.update(|g| *g += 1);

        #[cfg(feature = "hydrate")]
        {
            let my_generation = generation.get_untracked();
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(DEBOUNCE_MS).await;
                if generation.get_untracked() == my_generation {
                    debounced_query.set(value);
                }
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = value;
    };

    let results = Resource::new(move || debounced_query.get(), |q| async move {
        if q.trim().len() < 2 {
            Ok(Vec::new())
        } else {
            search_users(q).await
        }
    });

    view! {
        <div class="flex flex-col gap-3">
            <input
                type="text"
                placeholder="Search by username…"
                prop:value=query
                on:input=on_input
                class="w-full px-4 py-2.5 rounded-xl bg-zinc-900 border border-zinc-800 text-white placeholder-zinc-600 focus:outline-none focus:border-zinc-600"
            />
            <Show when=move || !debounced_query.get().trim().is_empty()>
                <Transition fallback=|| ()>
                    {move || {
                        let rows = results.get().and_then(|r| r.ok()).unwrap_or_default();
                        if rows.is_empty() {
                            view! { <p class="text-zinc-500 text-sm italic px-1">"No users found"</p> }.into_any()
                        } else {
                            view! {
                                <div class="flex flex-col divide-y divide-zinc-800 rounded-2xl bg-zinc-900 border border-zinc-800 overflow-hidden">
                                    {rows.into_iter().map(|r| view! { <SearchResultRow result=r/> }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
                </Transition>
            </Show>
        </div>
    }
}
