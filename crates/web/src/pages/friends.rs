use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::{lazy_route, LazyRoute};

use crate::components::{FriendSearch, FriendsList};
use crate::friends::{get_friends_overview, use_friends_presence};

/// Standalone friends page.
///
/// The friends list and user search used to exist only nested inside your
/// own `/u/:username` profile, which meant the only way to notice a pending
/// request was to go and look for it. This gives them a real home that the
/// nav can link to and badge.
#[derive(Clone)]
pub struct FriendsPage;

#[lazy_route]
impl LazyRoute for FriendsPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let presence = use_friends_presence();
        // Refetches whenever the socket reports a change, so accepting a
        // request from the toast updates this list without a reload.
        let overview = Resource::new(
            move || presence.list_trigger.get(),
            |_| async move { get_friends_overview().await },
        );

        let fallback = || {
            view! {
                <div class="flex flex-col gap-3">
                    <div class="h-10 rounded-control skeleton-shimmer"/>
                    <div class="h-32 rounded-card skeleton-shimmer"/>
                </div>
            }
        };

        view! {
            <Title text="Friends"/>
            <div class="px-6 py-8 max-w-2xl mx-auto flex flex-col gap-6">
                <h1 class="display-1 text-white">"Friends"</h1>
                <Transition fallback=fallback>
                    {move || overview.get().map(|result| match result {
                        Err(e) => view! {
                            <p class="text-zinc-500 text-sm">
                                "Couldn't load your friends: " {e.to_string()}
                            </p>
                        }.into_any(),
                        Ok(overview) => view! {
                            <div class="flex flex-col gap-4">
                                <FriendSearch/>
                                <FriendsList
                                    friends=overview.friends.clone()
                                    is_own=true
                                    incoming=overview.incoming_requests.clone()
                                    outgoing=overview.outgoing_requests.clone()
                                />
                            </div>
                        }.into_any(),
                    })}
                </Transition>
            </div>
        }
        .into_any()
    }
}
