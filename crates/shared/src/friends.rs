use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Minimal display identity for a friend/search result. Deliberately
/// narrower than `PlayerInfo` (which carries a per-`Category` rating and so
/// needs a `ratings` join) — a friends list just needs enough to render an
/// avatar and a name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendSummary {
    pub id: Uuid,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
}

/// A friend's currently in-progress game, as seen by a bystander. Distinct
/// from `ActiveGame`, whose `my_side`/`opponent` fields are relative to the
/// querying user — here the querying user isn't a participant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendActiveGame {
    /// Doubles as the spectate link target (`/game/{game_id}`) and the
    /// reason the Challenge button is disabled.
    pub game_id: Uuid,
    pub opponent_username: Option<String>,
}

/// One row in the friends list. `online` and `in_game` are computed
/// server-side per request (from the in-memory presence map and one batched
/// `games` query respectively) — neither is stored anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRow {
    pub user: FriendSummary,
    pub online: bool,
    pub in_game: Option<FriendActiveGame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FriendRelation {
    None,
    PendingOutgoing,
    PendingIncoming,
    Friends,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSearchResult {
    pub user: FriendSummary,
    pub relation: FriendRelation,
}

/// Everything a `/u/:username` profile page needs about the *relationship
/// and social graph* side, in one round trip. Ratings and recent games are
/// deliberately not bundled here — those are fetched by `EloCard`/
/// `RecentGames`, which already know how to load an arbitrary user's data
/// and are reused as-is on the profile page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileView {
    pub user: FriendSummary,
    pub bio: Option<String>,
    pub country: Option<String>,
    /// Whether the viewer is looking at their own profile — gates the
    /// settings section and the incoming/outgoing request lists (visible
    /// only to their owner; the friends list itself is public).
    pub is_own: bool,
    /// The viewer's relationship to this profile's owner. Meaningless (left
    /// as `None`) when `is_own` is true.
    pub relation: FriendRelation,
    pub online: bool,
    pub in_game: Option<FriendActiveGame>,
    pub friends: Vec<FriendRow>,
    pub incoming_requests: Vec<FriendSummary>,
    pub outgoing_requests: Vec<FriendSummary>,
}
