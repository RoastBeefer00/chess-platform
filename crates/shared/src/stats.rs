//! Per-category player statistics, for the stats page behind each rating
//! card.

use serde::{Deserialize, Serialize};

/// One point on the rating-over-time chart.
///
/// `at` is epoch milliseconds rather than a `time::OffsetDateTime` so this
/// crate stays free of a date-time dependency and compiles to WASM as
/// cheaply as it does natively — the client only ever needs to order these
/// and space them along an axis, never to format a calendar date from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RatingPoint {
    pub at: i64,
    pub rating: i32,
}

/// A win/draw/loss record, as counted for one side of the board.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorRecord {
    pub wins: i64,
    pub draws: i64,
    pub losses: i64,
}

impl ColorRecord {
    pub fn total(&self) -> i64 {
        self.wins + self.draws + self.losses
    }

    /// Wins as a percentage of games played, counting a draw as half a win —
    /// the usual chess convention, and the one that makes the two colours
    /// comparable when they have different draw rates. `None` for a side
    /// that has never been played, which renders as a dash rather than as a
    /// misleading 0%.
    pub fn score_rate(&self) -> Option<f64> {
        let total = self.total();
        if total == 0 {
            return None;
        }
        Some((self.wins as f64 + self.draws as f64 / 2.0) / total as f64 * 100.0)
    }
}

/// Everything the stats page shows for one `Category`, in one round trip.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CategoryStats {
    /// Oldest first, so the chart can plot it without re-sorting.
    pub rating_points: Vec<RatingPoint>,
    pub as_white: ColorRecord,
    pub as_black: ColorRecord,
    /// The current rating, which is not necessarily the last history point:
    /// `ratings` carries the live value and `rating_history` only gains a row
    /// when a rated game ends, so a player with no rated games in this
    /// category has a rating but no history at all.
    pub current_rating: i32,
    pub peak_rating: Option<i32>,
}

impl CategoryStats {
    /// The combined record across both colours.
    pub fn overall(&self) -> ColorRecord {
        ColorRecord {
            wins: self.as_white.wins + self.as_black.wins,
            draws: self.as_white.draws + self.as_black.draws,
            losses: self.as_white.losses + self.as_black.losses,
        }
    }
}
