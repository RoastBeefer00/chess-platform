use leptos::prelude::*;
use shared::{FriendRelation, RatingMode, TimeControl, UserSearchResult};
use uuid::Uuid;

/// Shared auth guard for every friends/challenge endpoint: requires a signed
/// in, onboarded, non-guest user. Guests are excluded from the whole
/// feature — every "Continue as Guest" click makes a real, permanent `users`
/// row, and letting them accumulate friends/requests would pollute search
/// and friend lists with throwaway accounts.
#[cfg(feature = "ssr")]
async fn current_friends_user() -> Result<shared::FriendSummary, ServerFnError> {
    use crate::auth::AuthBackend;
    use axum_login::AuthSession;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let user = auth.user.ok_or_else(|| ServerFnError::new("unauthenticated"))?;
    if user.username.is_none() {
        return Err(ServerFnError::new("complete onboarding first"));
    }
    if user.is_guest {
        return Err(ServerFnError::new("guests can't use friends"));
    }
    Ok(shared::FriendSummary { id: user.id, username: user.username, avatar_url: user.avatar_url })
}

/// Everything a `/u/:username` profile page needs about the social-graph
/// side in one round trip. Ratings and recent games are fetched separately
/// by `EloCard`/`RecentGames`, which already know how to load an arbitrary
/// user's data.
#[server]
pub async fn get_profile(username: String) -> Result<shared::ProfileView, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;
    use shared::{FriendActiveGame, FriendRow, FriendSummary, ProfileView};

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let viewer_id = auth.user.map(|u| u.id).ok_or_else(|| ServerFnError::new("unauthenticated"))?;
    let state = expect_context::<AppState>();

    let target = state
        .user_store
        .find_by_username(&username)
        .await?
        .ok_or_else(|| ServerFnError::new("user not found"))?;
    let is_own = target.id == viewer_id;

    let relation = if is_own {
        FriendRelation::None
    } else {
        state.friend_store.relation(viewer_id, target.id).await?
    };

    let online = state.is_online(&target.id).await;
    let in_game = state
        .game_store
        .find_active_game(target.id)
        .await?
        .map(|g| FriendActiveGame { game_id: g.id, opponent_username: g.opponent.username });

    let friends = state.friend_store.list_friends(target.id).await?;
    let friend_ids: Vec<Uuid> = friends.iter().map(|f| f.id).collect();
    let friends_online = state.online_among(&friend_ids).await;
    let mut friends_in_game = state.game_store.active_games_for(&friend_ids).await?;
    let friends = friends
        .into_iter()
        .map(|f| {
            let online = friends_online.contains(&f.id);
            let in_game = friends_in_game.remove(&f.id);
            FriendRow { user: f, online, in_game }
        })
        .collect();

    let (incoming_requests, outgoing_requests) = if is_own {
        (
            state.friend_store.list_pending_received(target.id).await?,
            state.friend_store.list_pending_sent(target.id).await?,
        )
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(ProfileView {
        user: FriendSummary { id: target.id, username: target.username, avatar_url: target.avatar_url },
        bio: target.bio,
        country: target.country,
        is_own,
        relation,
        online,
        in_game,
        friends,
        incoming_requests,
        outgoing_requests,
    })
}

#[server]
pub async fn search_users(query: String) -> Result<Vec<UserSearchResult>, ServerFnError> {
    use crate::state::AppState;

    let query = query.trim();
    if query.len() < 2 {
        return Ok(Vec::new());
    }
    let me = current_friends_user().await?;
    let state = expect_context::<AppState>();

    let results = state.friend_store.search_users(query, me.id, 20).await?;
    Ok(results.into_iter().map(|(user, relation)| UserSearchResult { user, relation }).collect())
}

#[server]
pub async fn send_friend_request(target_id: Uuid) -> Result<FriendRelation, ServerFnError> {
    use crate::db::SendRequestOutcome;
    use crate::state::AppState;
    use shared::FriendsServerMessage;

    let me = current_friends_user().await?;
    if me.id == target_id {
        return Err(ServerFnError::new("can't friend yourself"));
    }
    let state = expect_context::<AppState>();

    let outcome = state.friend_store.send_request(me.id, target_id).await?;
    let relation = match outcome {
        SendRequestOutcome::Created => FriendRelation::PendingOutgoing,
        SendRequestOutcome::AutoAccepted | SendRequestOutcome::AlreadyFriends => FriendRelation::Friends,
        SendRequestOutcome::AlreadyPending => FriendRelation::PendingOutgoing,
    };
    state.notify_friend(target_id, FriendsServerMessage::FriendListChanged).await;
    Ok(relation)
}

#[server]
pub async fn respond_friend_request(requester_id: Uuid, accept: bool) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use shared::FriendsServerMessage;

    let me = current_friends_user().await?;
    let state = expect_context::<AppState>();

    if accept {
        state.friend_store.accept_request(me.id, requester_id).await?;
    } else {
        state.friend_store.decline_request(me.id, requester_id).await?;
    }
    state.notify_friend(requester_id, FriendsServerMessage::FriendListChanged).await;
    Ok(())
}

#[server]
pub async fn cancel_friend_request(target_id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use shared::FriendsServerMessage;

    let me = current_friends_user().await?;
    let state = expect_context::<AppState>();

    state.friend_store.cancel_request(me.id, target_id).await?;
    state.notify_friend(target_id, FriendsServerMessage::FriendListChanged).await;
    Ok(())
}

#[server]
pub async fn unfriend(other_id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use shared::FriendsServerMessage;

    let me = current_friends_user().await?;
    let state = expect_context::<AppState>();

    state.friend_store.remove_friend(me.id, other_id).await?;
    state.notify_friend(other_id, FriendsServerMessage::FriendListChanged).await;
    Ok(())
}

#[server]
pub async fn send_challenge(
    target_id: Uuid,
    time_control: TimeControl,
    rating_mode: RatingMode,
) -> Result<Uuid, ServerFnError> {
    use crate::state::AppState;
    use shared::FriendsServerMessage;

    let me = current_friends_user().await?;
    let state = expect_context::<AppState>();

    if !state.friend_store.are_friends(me.id, target_id).await? {
        return Err(ServerFnError::new("not friends"));
    }
    if !state.is_online(&target_id).await {
        return Err(ServerFnError::new("friend is offline"));
    }
    if state.game_store.find_active_game(target_id).await?.is_some() {
        return Err(ServerFnError::new("friend is already in a game"));
    }
    // Symmetric guard — without this, a mid-game challenger who gets
    // accepted ends up with two 'active' rows, which breaks
    // `find_active_game`'s "at most one active game per user" assumption
    // (the reconnect feature's `LIMIT 1` query).
    if state.game_store.find_active_game(me.id).await?.is_some() {
        return Err(ServerFnError::new("finish your current game first"));
    }

    // One outstanding challenge per challenger.
    for cancelled in state.cancel_challenges_from(&me.id).await {
        state
            .notify_friend(cancelled.to, FriendsServerMessage::ChallengeCancelled { challenge_id: cancelled.id })
            .await;
    }

    let challenge = crate::state::PendingChallenge::new(me.id, me.clone(), target_id, time_control.clone(), rating_mode);
    let challenge_id = challenge.id;
    state.insert_challenge(challenge).await;

    state
        .notify_friend(
            target_id,
            FriendsServerMessage::ChallengeReceived { challenge_id, from: me, time_control, rating_mode },
        )
        .await;

    Ok(challenge_id)
}

#[server]
pub async fn respond_challenge(challenge_id: Uuid, accept: bool) -> Result<Option<Uuid>, ServerFnError> {
    use crate::state::AppState;
    use shared::FriendsServerMessage;
    use shared::{GameConfig, Variant};

    let me = current_friends_user().await?;
    let state = expect_context::<AppState>();

    let challenge = state
        .take_challenge(&challenge_id)
        .await
        .ok_or_else(|| ServerFnError::new("challenge expired or already answered"))?;
    if challenge.to != me.id {
        // Not ours to answer — put it back so the rightful addressee can
        // still respond.
        state.insert_challenge(challenge).await;
        return Err(ServerFnError::new("not your challenge"));
    }

    if !accept {
        state
            .notify_friend(challenge.from, FriendsServerMessage::ChallengeDeclined { challenge_id })
            .await;
        return Ok(None);
    }

    // TOCTOU: either side could have started a game in the 60s the
    // challenge was outstanding.
    if state.game_store.find_active_game(challenge.from).await?.is_some()
        || state.game_store.find_active_game(me.id).await?.is_some()
    {
        state
            .notify_friend(challenge.from, FriendsServerMessage::ChallengeCancelled { challenge_id })
            .await;
        return Err(ServerFnError::new("no longer available — someone started a game"));
    }

    // Coin flip for side, same as matchmaking pairing.
    let (white, black) = if rand::random::<bool>() { (challenge.from, me.id) } else { (me.id, challenge.from) };
    let game_config =
        GameConfig { time_control: challenge.time_control, variant: Variant::Standard, rated: challenge.rating_mode };
    let game_id = state.create_game(game_config, white, black, (0.0, 0.0)).await?;

    state
        .notify_friend(challenge.from, FriendsServerMessage::ChallengeAccepted { challenge_id, game_id })
        .await;
    // Clears the toast on the acceptor's *other* tabs.
    state.notify_friend(me.id, FriendsServerMessage::ChallengeCancelled { challenge_id }).await;

    Ok(Some(game_id))
}

#[server]
pub async fn cancel_challenge(challenge_id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use shared::FriendsServerMessage;

    let me = current_friends_user().await?;
    let state = expect_context::<AppState>();

    if let Some(challenge) = state.take_challenge(&challenge_id).await {
        if challenge.from != me.id {
            state.insert_challenge(challenge).await;
            return Err(ServerFnError::new("not your challenge"));
        }
        state
            .notify_friend(challenge.to, FriendsServerMessage::ChallengeCancelled { challenge_id })
            .await;
    }
    Ok(())
}
