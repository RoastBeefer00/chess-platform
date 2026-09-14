use leptos::prelude::*;
use shared::{MoveCheck, PuzzleSummary};

/// `themes` is space-separated, any-of (empty = no theme filter, i.e. all
/// puzzles) — a plain `String` rather than `Vec<String>` deliberately: the
/// default server-fn arg encoding drops an *empty* `Vec` from the request
/// body entirely rather than sending it as present-but-empty, which the
/// server then reports as a missing field. An empty string has no such
/// ambiguity, and it's the same space-separated convention the `themes`
/// column itself already uses.
#[server]
pub async fn get_random_puzzle(
    themes: String,
    min_rating: i32,
    max_rating: i32,
) -> Result<PuzzleSummary, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let themes: Vec<String> = themes.split_whitespace().map(String::from).collect();
    state
        .puzzle_store
        .random(&themes, min_rating, max_rating)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server]
pub async fn check_puzzle_move(
    id: String,
    ply: usize,
    uci: String,
) -> Result<MoveCheck, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    state
        .puzzle_store
        .check_move(&id, ply, &uci)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// The correct move (UCI) at `ply`, revealed only on an explicit hint
/// request — everything else about the solution stays server-side.
#[server]
pub async fn get_puzzle_hint(id: String, ply: usize) -> Result<Option<String>, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    state
        .puzzle_store
        .hint(&id, ply)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}
