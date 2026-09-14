use leptos::prelude::*;
use shared::{MoveCheck, PuzzleSummary};

#[server]
pub async fn get_random_puzzle() -> Result<PuzzleSummary, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    state
        .puzzle_store
        .random()
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
