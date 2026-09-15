use std::collections::HashMap;

use leptos::prelude::*;
use shared::{RatingMode, TimeControl};
use uuid::Uuid;

/// An incoming challenge, as shown by the app-root toast.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingChallenge {
    pub challenge_id: Uuid,
    pub from_username: String,
    pub time_control: TimeControl,
    pub rating_mode: RatingMode,
}

/// An outgoing challenge this tab sent, awaiting a response.
#[derive(Debug, Clone, PartialEq)]
pub struct OutgoingChallenge {
    pub challenge_id: Uuid,
    pub to_username: String,
}

/// Signals fed by the app-wide friends-presence socket. Newtype so
/// `use_context` can find it by type, same shape as `CurrentUserResource`.
#[derive(Copy, Clone)]
pub struct FriendsPresence {
    /// Online-status *overrides*, empty until the socket delivers its first
    /// snapshot. A friends-list row should render
    /// `overrides.get(&id).copied().unwrap_or(row.online)` — the
    /// server-computed `row.online` is what SSR and the first hydrate paint
    /// agree on; this signal only overrides it once presence data arrives,
    /// so hydration doesn't flip every dot the instant it connects.
    pub online: RwSignal<HashMap<Uuid, bool>>,
    /// `Some(game_id)` overrides — a friend just started a game (or `None`,
    /// if that's ever wired up as "just ended").
    pub in_game: RwSignal<HashMap<Uuid, Option<Uuid>>>,
    /// At most one visible at a time — a second arrival replaces the first.
    pub incoming: RwSignal<Option<IncomingChallenge>>,
    pub outgoing: RwSignal<Option<OutgoingChallenge>>,
    /// Set when an outgoing challenge was accepted. A component rendered
    /// under `<Router>` (`ChallengeToast`) watches this and calls
    /// `use_navigate()` — navigation itself can't happen here, since this
    /// module's code runs in `App`'s scope, outside `<Router>`'s own
    /// context, the same reason `use_start_matchmaking` lives in a
    /// component rather than a plain module function.
    pub navigate_to_game: RwSignal<Option<Uuid>>,
    /// Bump to force the friends-list resource to refetch, same idiom as
    /// `AuthTrigger`.
    pub list_trigger: RwSignal<u64>,
}

pub fn use_friends_presence() -> FriendsPresence {
    use_context::<FriendsPresence>().expect("provide_friends_presence must be called at the App root")
}

/// Call once at the App root, after `provide_current_user()`. Opens (and,
/// under `hydrate`, keeps alive for the whole tab) the presence websocket;
/// under SSR this only provides the empty signals so server-rendered markup
/// has something to read.
pub fn provide_friends_presence() {
    let presence = FriendsPresence {
        online: RwSignal::new(HashMap::new()),
        in_game: RwSignal::new(HashMap::new()),
        incoming: RwSignal::new(None),
        outgoing: RwSignal::new(None),
        navigate_to_game: RwSignal::new(None),
        list_trigger: RwSignal::new(0),
    };
    provide_context(presence);

    #[cfg(feature = "hydrate")]
    {
        use futures::channel::mpsc;
        use futures::StreamExt;
        use gloo_timers::future::TimeoutFuture;
        use leptos::task::spawn_local;
        use shared::FriendsClientMessage;

        use crate::components::auth::use_current_user;
        use crate::friends::friends_websocket;

        // Guards against `use_auth_trigger()` bumps (e.g. the settings page
        // saving a preference) re-firing this effect and opening a second
        // connection — the resource it reads changes identity-wise on every
        // bump, but the socket must only ever start once per tab.
        let started = StoredValue::new(false);

        Effect::new(move |_| {
            if started.get_value() {
                return;
            }
            let Some(Ok(Some(_))) = use_current_user().get() else { return };
            started.set_value(true);

            spawn_local(async move {
                let mut backoff_ms = 500u32;
                loop {
                    let (mut tx, rx) = mpsc::channel::<FriendsClientMessage>(1);
                    let _ = tx.try_send(FriendsClientMessage::Connect);

                    match friends_websocket(rx.map(Ok).into()).await {
                        Ok(mut messages) => {
                            backoff_ms = 500;
                            while let Some(msg) = messages.next().await {
                                let Ok(msg) = msg else { continue };
                                handle_message(&presence, msg);
                            }
                        }
                        Err(e) => leptos::logging::warn!("friends websocket error: {e}"),
                    }

                    TimeoutFuture::new(backoff_ms).await;
                    backoff_ms = (backoff_ms * 2).min(5000);
                }
            });
        });
    }
}

#[cfg(feature = "hydrate")]
fn handle_message(presence: &FriendsPresence, msg: shared::FriendsServerMessage) {
    use shared::FriendsServerMessage;

    match msg {
        FriendsServerMessage::OnlineSnapshot { online } => {
            presence.online.set(online.into_iter().map(|id| (id, true)).collect());
        }
        FriendsServerMessage::PresenceUpdate { user_id, online } => {
            presence.online.update(|map| {
                map.insert(user_id, online);
            });
        }
        FriendsServerMessage::InGameUpdate { user_id, game_id } => {
            presence.in_game.update(|map| {
                map.insert(user_id, game_id);
            });
        }
        FriendsServerMessage::ChallengeReceived { challenge_id, from, time_control, rating_mode } => {
            presence.incoming.set(Some(IncomingChallenge {
                challenge_id,
                from_username: from.username.unwrap_or_else(|| "Anonymous".to_string()),
                time_control,
                rating_mode,
            }));
        }
        FriendsServerMessage::ChallengeCancelled { challenge_id } => {
            if presence.incoming.get_untracked().is_some_and(|c| c.challenge_id == challenge_id) {
                presence.incoming.set(None);
            }
            if presence.outgoing.get_untracked().is_some_and(|c| c.challenge_id == challenge_id) {
                presence.outgoing.set(None);
            }
        }
        FriendsServerMessage::ChallengeDeclined { challenge_id } => {
            if presence.outgoing.get_untracked().is_some_and(|c| c.challenge_id == challenge_id) {
                presence.outgoing.set(None);
            }
        }
        FriendsServerMessage::ChallengeAccepted { challenge_id, game_id } => {
            if presence.outgoing.get_untracked().is_some_and(|c| c.challenge_id == challenge_id) {
                presence.outgoing.set(None);
                presence.navigate_to_game.set(Some(game_id));
            }
        }
        FriendsServerMessage::FriendListChanged => {
            presence.list_trigger.update(|v| *v += 1);
        }
    }
}
