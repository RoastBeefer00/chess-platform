use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use shared::{NewTournament, PairingResult, TournamentSummary, TournamentView};
use uuid::Uuid;

/// The tournament list, with the server's clock alongside so the page can
/// count down to start times without trusting the browser's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TournamentList {
    pub tournaments: Vec<TournamentSummary>,
    pub server_now: i64,
}

/// Tournaments are public and free to create, so each account gets a small
/// budget: this many open at once (a club night needs one), and this many
/// new ones per day. It just stops the list being flooded.
#[cfg(feature = "ssr")]
const MAX_OPEN_TOURNAMENTS_PER_ORGANIZER: i64 = 3;
#[cfg(feature = "ssr")]
const MAX_TOURNAMENTS_PER_DAY: i64 = 10;

/// Whoever is asking, if they are signed in. Tournament pages are public, so
/// unlike most server fns here "nobody" is a valid answer.
#[cfg(feature = "ssr")]
async fn viewer() -> Result<Option<crate::auth::User>, ServerFnError> {
    use crate::auth::AuthBackend;
    use axum_login::AuthSession;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    Ok(auth.user)
}

/// A signed-in, onboarded, non-guest user — required to enter or organize.
///
/// Guests are excluded for the reason they are excluded from rated play: a
/// guest account is one click and free, so a tournament that admits them can
/// be filled with (or won by) as many throwaway entrants as someone cares
/// to make.
#[cfg(feature = "ssr")]
async fn member() -> Result<crate::auth::User, ServerFnError> {
    let user = viewer().await?.ok_or_else(|| ServerFnError::new("sign in to do that"))?;
    if user.username.is_none() {
        return Err(ServerFnError::new("complete onboarding first"));
    }
    if user.is_guest {
        return Err(ServerFnError::new("create an account to take part in tournaments"));
    }
    Ok(user)
}

/// Loads a tournament and checks the caller organizes it.
#[cfg(feature = "ssr")]
async fn organized_by_caller(
    state: &crate::state::AppState,
    id: Uuid,
) -> Result<crate::db::TournamentRow, ServerFnError> {
    let user = member().await?;
    let row = state
        .tournament_store
        .get(id, Some(user.id))
        .await?
        .ok_or_else(|| ServerFnError::new("tournament not found"))?;
    if row.organizer_id != Some(user.id) {
        return Err(ServerFnError::new("only the organizer can do that"));
    }
    Ok(row)
}

#[server]
pub async fn list_tournaments() -> Result<TournamentList, ServerFnError> {
    use crate::state::AppState;
    use crate::tournament::service;

    let state = expect_context::<AppState>();
    let viewer_id = viewer().await?.map(|u| u.id);

    service::ensure_schedule(&state).await;
    state.tournament_store.sync_arena_status().await?;
    let rows = state.tournament_store.list(viewer_id).await?;
    Ok(TournamentList {
        tournaments: rows.iter().map(|r| r.summary()).collect(),
        server_now: time::OffsetDateTime::now_utc().unix_timestamp(),
    })
}

/// The tournament page's whole state. Polled while the tournament is live —
/// see `service::build_view` for why the poll itself matters to an arena.
#[server]
pub async fn get_tournament(id: Uuid) -> Result<TournamentView, ServerFnError> {
    use crate::state::AppState;
    use crate::tournament::service;

    let state = expect_context::<AppState>();
    let user = viewer().await?;
    service::build_view(&state, id, user.as_ref()).await
}

#[server]
pub async fn create_tournament(new: NewTournament) -> Result<Uuid, ServerFnError> {
    use crate::state::AppState;
    use shared::TournamentFormat;

    let user = member().await?;
    new.validate().map_err(ServerFnError::new)?;
    let state = expect_context::<AppState>();

    if state.tournament_store.count_open_by_organizer(user.id).await? >= MAX_OPEN_TOURNAMENTS_PER_ORGANIZER {
        return Err(ServerFnError::new(
            "you already have several tournaments open — finish or cancel one first",
        ));
    }

    let now = time::OffsetDateTime::now_utc();
    if state.tournament_store.count_created_since(user.id, now - time::Duration::days(1)).await? >= MAX_TOURNAMENTS_PER_DAY {
        return Err(ServerFnError::new("you've created a lot of tournaments today — try again tomorrow"));
    }
    let starts_at = match new.format {
        TournamentFormat::Arena => now + time::Duration::minutes(i64::from(new.starts_in_minutes)),
        // Round-based events start when the organizer says so; until then
        // this is just "when it was announced".
        _ => now,
    };
    let id = state.tournament_store.create(user.id, &new, starts_at).await?;
    tracing::info!(tournament_id = %id, organizer = %user.id, format = new.format.as_str(), "tournament_created");
    Ok(id)
}

#[server]
pub async fn join_tournament(id: Uuid) -> Result<(), ServerFnError> {
    use crate::db::JoinOutcome;
    use crate::state::AppState;
    use shared::{TournamentFormat, TournamentStatus, TournamentVenue};

    let user = member().await?;
    let state = expect_context::<AppState>();
    state.tournament_store.sync_arena_status().await?;
    let row = state
        .tournament_store
        .get(id, Some(user.id))
        .await?
        .ok_or_else(|| ServerFnError::new("tournament not found"))?;

    if row.venue() == TournamentVenue::Otb {
        return Err(ServerFnError::new("the organizer enters players for an over-the-board event"));
    }
    match (row.format(), row.status()) {
        // An arena is a drop-in: join whenever, as long as it isn't over.
        (TournamentFormat::Arena, TournamentStatus::Upcoming | TournamentStatus::Live) => {}
        (_, TournamentStatus::Upcoming) => {}
        (_, TournamentStatus::Live) => {
            return Err(ServerFnError::new("this tournament has already started"));
        }
        _ => return Err(ServerFnError::new("this tournament is over")),
    }

    match state.tournament_store.join(id, user.id).await? {
        JoinOutcome::Joined(_) => Ok(()),
        JoinOutcome::Full => Err(ServerFnError::new("this tournament is full")),
    }
}

/// Leaves a tournament. Before it starts that removes the entry entirely;
/// once there are results it only stops further pairings, so the games
/// already played still stand for everyone else.
#[server]
pub async fn withdraw_from_tournament(id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use shared::TournamentStatus;

    let user = member().await?;
    let state = expect_context::<AppState>();
    let row = state
        .tournament_store
        .get(id, Some(user.id))
        .await?
        .ok_or_else(|| ServerFnError::new("tournament not found"))?;
    let players = state.tournament_store.players(id).await?;
    let Some(me) = players.iter().find(|p| p.user_id == Some(user.id)) else {
        return Ok(());
    };

    match row.status() {
        TournamentStatus::Upcoming => {
            state.tournament_store.remove_player(id, me.id).await?;
        }
        TournamentStatus::Live => {
            state.tournament_store.set_withdrawn(id, me.id, true).await?;
        }
        _ => {}
    }
    state.redis_client.lobby_leave(id, user.id).await;
    Ok(())
}

#[server]
pub async fn start_tournament(id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use crate::tournament::service;

    let state = expect_context::<AppState>();
    let row = organized_by_caller(&state, id).await?;
    service::start(&state, &row).await
}

/// Pairs the next round, or finishes the tournament if the last round is
/// complete.
#[server]
pub async fn next_tournament_round(id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use crate::tournament::service;

    let state = expect_context::<AppState>();
    let row = organized_by_caller(&state, id).await?;
    service::open_next_round(&state, &row).await
}

/// Nudges an online friend about a tournament: a toast on whichever of their
/// tabs are open. Only for friends who are online — the push isn't stored,
/// so an offline friend would never see it — and once per friend per ten
/// minutes.
#[server]
pub async fn invite_to_tournament(id: Uuid, friend_id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use shared::{FriendSummary, FriendsServerMessage, TournamentVenue};

    let user = member().await?;
    let state = expect_context::<AppState>();
    let row = state
        .tournament_store
        .get(id, Some(user.id))
        .await?
        .ok_or_else(|| ServerFnError::new("tournament not found"))?;
    if row.status().is_over() {
        return Err(ServerFnError::new("this tournament is over"));
    }
    if row.venue() == TournamentVenue::Otb {
        return Err(ServerFnError::new("over-the-board players are added by the organizer"));
    }
    if !state.friend_store.are_friends(user.id, friend_id).await? {
        return Err(ServerFnError::new("you can only invite friends"));
    }
    if !state.is_online(&friend_id).await {
        return Err(ServerFnError::new("they're offline — send them the link instead"));
    }
    if !state.redis_client.tournament_invite_claim(id, user.id, friend_id).await {
        return Ok(());
    }
    let from = FriendSummary { id: user.id, username: user.username.clone(), avatar_url: user.avatar_url.clone() };
    state
        .notify_friend(friend_id, FriendsServerMessage::TournamentInvite { tournament_id: id, name: row.name.clone(), from })
        .await;
    Ok(())
}

/// After the last round of an over-the-board tournament: pairs the players
/// who are tied for a podium place in one extra round.
#[server]
pub async fn add_tiebreak_round(id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use crate::tournament::service;

    let state = expect_context::<AppState>();
    let row = organized_by_caller(&state, id).await?;
    service::open_tiebreak_round(&state, &row).await
}

/// Ends a round-based tournament where it stands.
#[server]
pub async fn finish_tournament(id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use shared::TournamentFormat;

    let state = expect_context::<AppState>();
    let row = organized_by_caller(&state, id).await?;
    if row.format() == TournamentFormat::Arena {
        return Err(ServerFnError::new("an arena ends when its time runs out"));
    }
    if !state.tournament_store.finish(id).await? {
        return Err(ServerFnError::new("this tournament isn't running"));
    }
    Ok(())
}

#[server]
pub async fn cancel_tournament(id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    organized_by_caller(&state, id).await?;
    if !state.tournament_store.cancel(id).await? {
        return Err(ServerFnError::new("only a tournament that hasn't started can be cancelled"));
    }
    Ok(())
}

/// Organizer-only: enter a player in an over-the-board tournament.
///
/// `name` is either a plain name for someone with no account here, or
/// `@username` to enter an existing account — which also puts the tournament
/// on that player's own list. The `@` is required for that on purpose: a
/// walk-in called "magnus" should not silently become the account `magnus`.
#[server]
pub async fn add_tournament_player(id: Uuid, name: String) -> Result<(), ServerFnError> {
    use crate::db::JoinOutcome;
    use crate::state::AppState;
    use shared::{TournamentFormat, TournamentStatus, TournamentVenue, PLAYER_NAME_MAX_LEN};

    let state = expect_context::<AppState>();
    let row = organized_by_caller(&state, id).await?;
    if row.venue() != TournamentVenue::Otb {
        return Err(ServerFnError::new("players enter an online tournament themselves"));
    }
    match (row.format(), row.status()) {
        (_, TournamentStatus::Upcoming) => {}
        // A swiss absorbs a late entry: they simply start on zero. A round
        // robin's schedule is fixed the moment it starts.
        (TournamentFormat::Swiss, TournamentStatus::Live) => {}
        (_, TournamentStatus::Live) => {
            return Err(ServerFnError::new("a round robin can't take late entries"));
        }
        _ => return Err(ServerFnError::new("this tournament is over")),
    }

    let name = name.trim();
    let outcome = if let Some(username) = name.strip_prefix('@') {
        let user = state
            .user_store
            .find_by_username(username.trim())
            .await?
            .filter(|u| !u.is_guest)
            .ok_or_else(|| ServerFnError::new("no account with that username"))?;
        state.tournament_store.join(id, user.id).await?
    } else {
        if name.is_empty() {
            return Err(ServerFnError::new("enter a name"));
        }
        if name.chars().count() > PLAYER_NAME_MAX_LEN {
            return Err(ServerFnError::new("that name is too long"));
        }
        state.tournament_store.add_named_player(id, name).await?
    };
    match outcome {
        JoinOutcome::Joined(_) => Ok(()),
        JoinOutcome::Full => Err(ServerFnError::new("this tournament is full")),
    }
}

/// Organizer-only: take a player out. Removes them before the start;
/// afterwards toggles their withdrawal, so a player who stepped out for a
/// round can be brought back.
#[server]
pub async fn remove_tournament_player(id: Uuid, player_id: Uuid) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use shared::TournamentStatus;

    let state = expect_context::<AppState>();
    let row = organized_by_caller(&state, id).await?;
    match row.status() {
        TournamentStatus::Upcoming => {
            state.tournament_store.remove_player(id, player_id).await?;
        }
        TournamentStatus::Live => {
            let players = state.tournament_store.players(id).await?;
            let Some(player) = players.iter().find(|p| p.id == player_id) else {
                return Err(ServerFnError::new("no such player"));
            };
            state.tournament_store.set_withdrawn(id, player_id, !player.withdrawn).await?;
        }
        _ => return Err(ServerFnError::new("this tournament is over")),
    }
    Ok(())
}

/// Organizer-only: enter or correct a result. `None` clears it.
///
/// For an over-the-board event this is how every result arrives. For an
/// online one it is the override — awarding a forfeit for an aborted game —
/// and the store refuses it while the game is still being played.
#[server]
pub async fn set_pairing_result(
    id: Uuid,
    pairing_id: Uuid,
    result: Option<PairingResult>,
) -> Result<(), ServerFnError> {
    use crate::state::AppState;
    use shared::{TournamentFormat, TournamentStatus, TournamentVenue};

    let state = expect_context::<AppState>();
    let row = organized_by_caller(&state, id).await?;
    if row.format() == TournamentFormat::Arena {
        return Err(ServerFnError::new("arena results can't be edited"));
    }
    if row.status() != TournamentStatus::Live {
        return Err(ServerFnError::new("results can only be changed while the tournament is running"));
    }
    let allowed = match result {
        Some(PairingResult::White | PairingResult::Black | PairingResult::Draw) => true,
        // Clearing only makes sense where the organizer is the source of
        // results. Online, "no result" means "game in progress".
        None => row.venue() == TournamentVenue::Otb,
        Some(PairingResult::Bye | PairingResult::Void) => false,
    };
    if !allowed {
        return Err(ServerFnError::new("that isn't a result a game can have"));
    }
    if !state.tournament_store.set_result(id, pairing_id, result).await? {
        return Err(ServerFnError::new("that game is still being played"));
    }
    Ok(())
}
