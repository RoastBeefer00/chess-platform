use leptos::prelude::*;
use shared::PuzzleSummary;

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

/// A specific puzzle by id, for a shared puzzle link (`/puzzles?id=...`) —
/// see `PuzzleStore::by_id`. No auth: `/puzzles` is a public page.
#[server]
pub async fn get_puzzle(id: String) -> Result<PuzzleSummary, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let result = state
        .puzzle_store
        .by_id(&id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("puzzle not found"));
    if let Err(ref e) = result {
        tracing::info!(%id, error = %e, "get_puzzle failed");
    }
    result
}
