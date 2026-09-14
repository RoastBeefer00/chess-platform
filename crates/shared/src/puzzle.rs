use serde::{Deserialize, Serialize};

/// Everything the client needs to render and attempt a puzzle. Deliberately
/// excludes the solution (`moves`) — that stays server-side and is checked
/// move-by-move via `check_puzzle_move`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PuzzleSummary {
    pub id: String,
    /// Position before the puzzle starts — `first_move` still needs to be
    /// auto-played from here to reach the actual solving position.
    pub fen: String,
    /// The opponent's setup move (UCI) that reaches the solving position
    /// from `fen` — shown as "opponent just moved," not part of the
    /// solution, so unlike the rest of `moves` it's safe to send up front.
    pub first_move: String,
    pub rating: i32,
    /// Space-separated tags, matching `public/images/puzzle-themes/*.svg`
    /// filenames.
    pub themes: String,
}

/// Result of checking a solver's move against the stored solution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveCheck {
    Correct {
        /// The opponent's forced reply to auto-play, if the puzzle isn't
        /// solved yet.
        reply: Option<String>,
        solved: bool,
    },
    Incorrect,
}
