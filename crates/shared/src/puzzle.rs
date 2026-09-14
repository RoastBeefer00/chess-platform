use serde::{Deserialize, Serialize};

/// Everything the client needs to render and solve a puzzle — including
/// the full solution. Mirrors lichess's own public puzzle API (its
/// `solution` field carries every move, not just the first): verifying
/// moves client-side means no network round trip per attempt, which
/// matters a lot on a slow connection. The trade-off is that a determined
/// user could read the solution from devtools, but a puzzle has no real
/// opponent or stakes to protect, so that's an acceptable trade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PuzzleSummary {
    pub id: String,
    /// Position before the puzzle starts — `moves[0]` still needs to be
    /// auto-played from here to reach the actual solving position.
    pub fen: String,
    /// Full solution, space-separated UCI. `moves[0]` is the opponent's
    /// setup move (auto-played to reach the solving position, not part of
    /// what the solver plays); from there the solver's and opponent's
    /// moves alternate, so the solver's `ply`-th move lives at
    /// `1 + 2*ply`.
    pub moves: String,
    pub rating: i32,
    /// Space-separated tags, matching `public/images/puzzle-themes/*.svg`
    /// filenames.
    pub themes: String,
}
