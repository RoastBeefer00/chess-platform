use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{RatingMode, Side, TimeControl};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchmakingServerMessage {
    Queued { time_control: TimeControl },
    Matched { game: Uuid, side: Side },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchmakingClientMessage {
    Join {
        time_control: TimeControl,
        rating_mode: RatingMode,
    },
}
