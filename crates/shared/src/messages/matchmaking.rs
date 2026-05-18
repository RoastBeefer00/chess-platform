use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Bucket, RatingMode, Side};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchmakingServerMessage {
    Queued { bucket: Bucket },
    Matched { game: Uuid, side: Side },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchmakingClientMessage {
    Join {
        bucket: Bucket,
        rating_mode: RatingMode,
    },
}
