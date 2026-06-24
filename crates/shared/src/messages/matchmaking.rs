use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{RatingMode, Side, TimeControl};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchmakingServerMessage {
    Queued { time_control: TimeControl },
    Matched { game: Uuid, side: Side },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchmakingClientMessage {
    Join {
        time_control: TimeControl,
        rating_mode: RatingMode,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RatingMode, Side, TimeControl, TimeMode};
    use uuid::Uuid;

    fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
        val: &T,
    ) {
        let json = serde_json::to_string(val).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*val, back);
    }

    #[test]
    fn server_queued_roundtrip() {
        let msg = MatchmakingServerMessage::Queued {
            time_control: TimeControl {
                initial_time: 600_000,
                mode: TimeMode::Increment(5_000),
            },
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_matched_roundtrip() {
        let msg = MatchmakingServerMessage::Matched {
            game: Uuid::new_v4(),
            side: Side::Black,
        };
        roundtrip(&msg);
    }

    #[test]
    fn client_join_roundtrip() {
        let msg = MatchmakingClientMessage::Join {
            time_control: TimeControl {
                initial_time: 300_000,
                mode: TimeMode::Delay(2_000),
            },
            rating_mode: RatingMode::Rated,
        };
        roundtrip(&msg);
    }
}
