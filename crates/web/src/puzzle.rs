use leptos::prelude::*;
use shared::PuzzleSummary;

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
