use leptos::prelude::*;
use server_fn::{codec::JsonEncoding, BoxedStream, Websocket};
use shared::{FriendsClientMessage, FriendsServerMessage};

#[server(protocol = Websocket<JsonEncoding, JsonEncoding>)]
pub async fn friends_websocket(
    input: BoxedStream<FriendsClientMessage, ServerFnError>,
) -> Result<BoxedStream<FriendsServerMessage, ServerFnError>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;
    use tokio_stream::StreamExt as _;
    use uuid::Uuid;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let user = auth.user.ok_or_else(|| ServerFnError::new("unauthenticated"))?;
    // Same defense-in-depth gate as the matchmaking socket: a no-username
    // user shouldn't be able to appear online or receive challenges.
    if user.username.is_none() {
        return Err(ServerFnError::new("complete onboarding first"));
    }
    // Guests are throwaway accounts (one per "Continue as Guest" click) —
    // excluding them from presence keeps friend lists/search free of noise.
    if user.is_guest {
        return Err(ServerFnError::new("guests can't use friends"));
    }
    let user_id = user.id;
    let state = expect_context::<AppState>();

    let (tx, rx) = futures::channel::mpsc::unbounded::<Result<FriendsServerMessage, ServerFnError>>();
    let session_id = Uuid::new_v4();

    let first_tab = state.add_friends_inbox(user_id, session_id, tx.clone()).await;

    let friend_ids = state.friend_store.list_friend_ids(user_id).await.unwrap_or_default();

    let online = state.online_among(&friend_ids).await.into_iter().collect();
    let _ = tx.unbounded_send(Ok(FriendsServerMessage::OnlineSnapshot { online }));

    // Seed the nav badge. Like `OnlineSnapshot` above, the incremental
    // pushes only ever report *changes*, so without this a tab that opens
    // with requests already waiting shows a zero badge until the next one
    // arrives.
    if let Ok(count) = state.friend_store.count_pending_received(user_id).await {
        let _ = tx.unbounded_send(Ok(FriendsServerMessage::PendingRequestCount { count }));
    }

    for challenge in state.challenges_for(&user_id).await {
        let _ = tx.unbounded_send(Ok(FriendsServerMessage::ChallengeReceived {
            challenge_id: challenge.id,
            from: challenge.from_summary,
            time_control: challenge.time_control,
            rating_mode: challenge.rating_mode,
        }));
    }

    if first_tab {
        for friend_id in &friend_ids {
            state
                .notify_friend(*friend_id, FriendsServerMessage::PresenceUpdate { user_id, online: true })
                .await;
        }
    }

    // Proves this tab is still here. Presence sessions age out if they stop
    // being refreshed (see `presence.lua`), which is what stops a process
    // that died without closing its connections from leaving people online
    // forever. A live connection has to say so periodically to stay counted.
    //
    // The task is owned by this connection: it exits as soon as the input
    // stream ends below, so it cannot outlive the tab it speaks for.
    let heartbeat = {
        let state = state.clone();
        let session = crate::state::friends_session_key_for(session_id);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(
                crate::state::FRIENDS_PRESENCE_HEARTBEAT_INTERVAL_SECS,
            ));
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let came_back = state.touch_friends_presence(user_id, &session).await;
                if came_back {
                    // A reader had pruned this user while a heartbeat ran
                    // late. They are plainly still here, so tell their
                    // friends they are back rather than leaving the two
                    // views disagreeing.
                    let friend_ids =
                        state.friend_store.list_friend_ids(user_id).await.unwrap_or_default();
                    for friend_id in friend_ids {
                        state
                            .notify_friend(
                                friend_id,
                                FriendsServerMessage::PresenceUpdate { user_id, online: true },
                            )
                            .await;
                    }
                }
            }
        })
    };

    tokio::spawn(async move {
        let mut input = input;
        // This connection carries no meaningful client->server traffic
        // beyond the implicit `Connect` — just block until the tab closes.
        while input.next().await.is_some() {}
        heartbeat.abort();

        // Re-query friend ids rather than reusing the connect-time list: a
        // friendship formed mid-session should still get notified when this
        // connection closes.
        let friend_ids = state.friend_store.list_friend_ids(user_id).await.unwrap_or_default();
        let was_last_tab = state.remove_friends_inbox(&user_id, session_id).await;

        if was_last_tab {
            for challenge in state.cancel_challenges_from(&user_id).await {
                state
                    .notify_friend(challenge.to, FriendsServerMessage::ChallengeCancelled {
                        challenge_id: challenge.id,
                    })
                    .await;
            }
            for friend_id in friend_ids {
                state
                    .notify_friend(friend_id, FriendsServerMessage::PresenceUpdate { user_id, online: false })
                    .await;
            }
        }
    });

    Ok(rx.into())
}
