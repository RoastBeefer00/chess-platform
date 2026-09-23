use leptos::prelude::*;
use shared::TimeMode;

use crate::friends::{CancelChallenge, RespondChallenge, RespondFriendRequest, use_friends_presence};

fn time_control_label(tc: &shared::TimeControl) -> String {
    let minutes = tc.initial_time / 60_000;
    match tc.mode {
        TimeMode::Increment(inc) => format!("{minutes}+{}", inc / 1000),
        TimeMode::Delay(_) => format!("{minutes}+0"),
    }
}

/// App-root toast stack for anything that arrives over the friends socket
/// and shouldn't wait for the user to go looking: incoming and outgoing
/// challenges, and incoming friend requests.
///
/// Rendered inside `<Router>` (unlike `provide_friends_presence`, which is
/// set up in `App` before the router exists) so it can call `use_navigate()`
/// when an outgoing challenge gets accepted from anywhere on the site.
///
/// Corner toasts rather than full-screen modals (see `ModalShell`'s
/// `fixed inset-0` pattern) — the recipient could be mid-game on
/// `/game/{id}`, and a modal would cover the board.
#[component]
pub fn Notifications() -> impl IntoView {
    let presence = use_friends_presence();
    let navigate = leptos_router::hooks::use_navigate();
    let navigate_acceptor = navigate.clone();

    Effect::new(move |_| {
        if let Some(game_id) = presence.navigate_to_game.get() {
            presence.navigate_to_game.set(None);
            navigate(&format!("/game/{game_id}"), Default::default());
        }
    });

    // Auto-dismiss incoming/outgoing after the server's 60s challenge TTL —
    // the server only sweeps expired challenges lazily (on the next
    // take/list), so a stale toast otherwise has no other reason to clear on
    // its own. Same idiom as `RematchControls`' `Declined` auto-reset.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(challenge) = presence.incoming.get() {
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(60_000).await;
                if presence.incoming.get_untracked().is_some_and(|c| c.challenge_id == challenge.challenge_id) {
                    presence.incoming.set(None);
                }
            });
        }
    });
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(challenge) = presence.outgoing.get() {
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(60_000).await;
                if presence.outgoing.get_untracked().is_some_and(|c| c.challenge_id == challenge.challenge_id) {
                    presence.outgoing.set(None);
                }
            });
        }
    });

    let respond = ServerAction::<RespondChallenge>::new();
    let cancel = ServerAction::<CancelChallenge>::new();
    let respond_request = ServerAction::<RespondFriendRequest>::new();

    // Dismiss the request toast once answered. The badge count and the
    // friends list both update from the server's own pushes, so nothing
    // local needs adjusting here.
    Effect::new(move |_| {
        if respond_request.value().get().is_some() {
            presence.incoming_request.set(None);
        }
    });

    // Friend requests don't expire the way challenges do, so this toast
    // dismisses on a timer purely so it doesn't sit in the corner forever —
    // the request itself is still waiting on the /friends page afterwards.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(request) = presence.incoming_request.get() {
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(15_000).await;
                if presence
                    .incoming_request
                    .get_untracked()
                    .is_some_and(|r| r.from_id == request.from_id)
                {
                    presence.incoming_request.set(None);
                }
            });
        }
    });

    Effect::new(move |_| {
        if respond.value().get().is_some() {
            presence.incoming.set(None);
        }
    });
    // The acceptor's own navigation — distinct from `ChallengeToast`'s other
    // effect (top of this component) that navigates the *challenger* when a
    // `ChallengeAccepted` push arrives over the friends websocket. Without
    // this, accepting only cleared the toast: nothing ever read the accepted
    // response's `Ok(Some(game_id))` to send the acceptor into their own
    // new game.
    Effect::new(move |_| {
        if let Some(Ok(Some(game_id))) = respond.value().get() {
            navigate_acceptor(&format!("/game/{game_id}"), Default::default());
        }
    });
    Effect::new(move |_| {
        if cancel.value().get().is_some() {
            presence.outgoing.set(None);
        }
    });

    view! {
        <div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2 max-w-xs">
            <Show when=move || presence.incoming.get().is_some()>
                {move || presence.incoming.get().map(|challenge| view! {
                    <div class="surface-card border-zinc-700 shadow-xl p-4 flex flex-col gap-3">
                        <div class="flex flex-col gap-0.5">
                            <span class="text-sm font-semibold text-white">
                                {challenge.from_username.clone()} " challenged you"
                            </span>
                            <span class="text-xs text-zinc-500">
                                {time_control_label(&challenge.time_control)}
                                {if challenge.rating_mode.is_rated() { " · Rated" } else { " · Casual" }}
                            </span>
                        </div>
                        <div class="flex gap-2">
                            <button
                                on:click=move |_| { respond.dispatch(RespondChallenge { challenge_id: challenge.challenge_id, accept: true }); }
                                class="flex-1 px-3 py-1.5 text-sm font-semibold bg-emerald-600 text-white rounded-control hover:bg-emerald-500 transition-colors cursor-pointer"
                            >
                                "Accept"
                            </button>
                            <button
                                on:click=move |_| { respond.dispatch(RespondChallenge { challenge_id: challenge.challenge_id, accept: false }); }
                                class="flex-1 px-3 py-1.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                            >
                                "Decline"
                            </button>
                        </div>
                    </div>
                })}
            </Show>

            <Show when=move || presence.incoming_request.get().is_some()>
                {move || presence.incoming_request.get().map(|request| view! {
                    <div class="surface-card border-zinc-700 shadow-xl p-4 flex flex-col gap-3">
                        <div class="flex flex-col gap-0.5">
                            <span class="text-sm font-semibold text-white">
                                {request.from_username.clone()} " sent you a friend request"
                            </span>
                        </div>
                        <div class="flex gap-2">
                            <button
                                on:click=move |_| { respond_request.dispatch(RespondFriendRequest { requester_id: request.from_id, accept: true }); }
                                class="flex-1 px-3 py-1.5 text-sm font-semibold bg-emerald-600 text-white rounded-control hover:bg-emerald-500 transition-colors cursor-pointer"
                            >
                                "Accept"
                            </button>
                            <button
                                on:click=move |_| { respond_request.dispatch(RespondFriendRequest { requester_id: request.from_id, accept: false }); }
                                class="flex-1 px-3 py-1.5 text-sm font-medium text-zinc-300 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                            >
                                "Decline"
                            </button>
                        </div>
                    </div>
                })}
            </Show>

            <Show when=move || presence.outgoing.get().is_some()>
                {move || presence.outgoing.get().map(|challenge| view! {
                    <div class="surface-card border-zinc-700 shadow-xl p-4 flex items-center justify-between gap-3">
                        <span class="text-sm text-zinc-300">
                            "Waiting for " {challenge.to_username.clone()} "…"
                        </span>
                        <button
                            on:click=move |_| { cancel.dispatch(CancelChallenge { challenge_id: challenge.challenge_id }); }
                            class="px-3 py-1.5 text-xs font-medium text-zinc-400 border border-zinc-700 rounded-control hover:border-zinc-500 hover:text-white transition-colors cursor-pointer"
                        >
                            "Cancel"
                        </button>
                    </div>
                })}
            </Show>
        </div>
    }
}
