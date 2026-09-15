use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{FriendSummary, RatingMode, TimeControl};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FriendsClientMessage {
    Connect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FriendsServerMessage {
    /// Sent once right after connect, so a fresh tab doesn't sit blank until
    /// the next presence transition — `PresenceUpdate` alone only ever
    /// reports *changes*, never the current state.
    OnlineSnapshot { online: Vec<Uuid> },
    /// A friend's first tab opened, or their last tab closed.
    PresenceUpdate { user_id: Uuid, online: bool },
    /// A friend entered (or left) an active game. Only ever pushed on entry
    /// in this version — see the `crates/web/src/state.rs::create_game` call
    /// site for why "left" isn't wired up yet.
    InGameUpdate { user_id: Uuid, game_id: Option<Uuid> },
    /// Someone challenged us. Replayed on reconnect for any still-live
    /// challenge, so a tab that was mid-reload when the push fired still
    /// sees it.
    ChallengeReceived {
        challenge_id: Uuid,
        from: FriendSummary,
        time_control: TimeControl,
        rating_mode: RatingMode,
    },
    /// The challenger cancelled, went offline, or the challenge expired.
    ChallengeCancelled { challenge_id: Uuid },
    /// Our outgoing challenge was declined.
    ChallengeDeclined { challenge_id: Uuid },
    /// Both parties navigate to `/game/{game_id}`. No `side` field — same
    /// reason `MatchmakingServerMessage::Matched`'s `side` goes unused on the
    /// client: orientation comes from `GameRoom::add_player`, not the push.
    ChallengeAccepted { challenge_id: Uuid, game_id: Uuid },
    /// A friend request arrived, was accepted, or a friend was removed.
    /// Deliberately payload-free — the client just refetches
    /// `friends_overview()` rather than maintaining a diff protocol.
    FriendListChanged,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TimeMode;

    fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
        val: &T,
    ) {
        let json = serde_json::to_string(val).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*val, back);
    }

    #[test]
    fn client_connect_roundtrip() {
        roundtrip(&FriendsClientMessage::Connect);
    }

    #[test]
    fn server_online_snapshot_roundtrip() {
        roundtrip(&FriendsServerMessage::OnlineSnapshot {
            online: vec![Uuid::new_v4(), Uuid::new_v4()],
        });
    }

    #[test]
    fn server_presence_update_roundtrip() {
        roundtrip(&FriendsServerMessage::PresenceUpdate {
            user_id: Uuid::new_v4(),
            online: true,
        });
    }

    #[test]
    fn server_in_game_update_roundtrip() {
        roundtrip(&FriendsServerMessage::InGameUpdate {
            user_id: Uuid::new_v4(),
            game_id: Some(Uuid::new_v4()),
        });
        roundtrip(&FriendsServerMessage::InGameUpdate {
            user_id: Uuid::new_v4(),
            game_id: None,
        });
    }

    #[test]
    fn server_challenge_received_roundtrip() {
        roundtrip(&FriendsServerMessage::ChallengeReceived {
            challenge_id: Uuid::new_v4(),
            from: FriendSummary {
                id: Uuid::new_v4(),
                username: Some("alice".to_string()),
                avatar_url: None,
            },
            time_control: TimeControl {
                initial_time: 300_000,
                mode: TimeMode::Increment(2_000),
            },
            rating_mode: RatingMode::Rated,
        });
    }

    #[test]
    fn server_challenge_cancelled_roundtrip() {
        roundtrip(&FriendsServerMessage::ChallengeCancelled {
            challenge_id: Uuid::new_v4(),
        });
    }

    #[test]
    fn server_challenge_declined_roundtrip() {
        roundtrip(&FriendsServerMessage::ChallengeDeclined {
            challenge_id: Uuid::new_v4(),
        });
    }

    #[test]
    fn server_challenge_accepted_roundtrip() {
        roundtrip(&FriendsServerMessage::ChallengeAccepted {
            challenge_id: Uuid::new_v4(),
            game_id: Uuid::new_v4(),
        });
    }

    #[test]
    fn server_friend_list_changed_roundtrip() {
        roundtrip(&FriendsServerMessage::FriendListChanged);
    }
}
