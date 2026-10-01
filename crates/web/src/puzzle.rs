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

/// Records how a puzzle went and returns the solver's updated stats.
///
/// `Ok(None)` for a signed-out visitor rather than an error: `/puzzles` is a
/// public page and solving without an account is fine, it just doesn't
/// accumulate anything.
///
/// Idempotent per puzzle (see `PuzzleStore::record_attempt`), so the client
/// doesn't have to remember whether it already reported this one.
#[server]
pub async fn record_puzzle_result(
    puzzle_id: String,
    solved: bool,
) -> Result<Option<shared::PuzzleStats>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let Some(user) = auth.user else { return Ok(None) };

    let state = expect_context::<AppState>();
    let stats = state
        .puzzle_store
        .record_attempt(user.id, &puzzle_id, solved)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(Some(stats))
}

/// The signed-in user's puzzle progress, for the page header. `Ok(None)`
/// when signed out, same reasoning as `record_puzzle_result`.
#[server]
pub async fn get_puzzle_stats() -> Result<Option<shared::PuzzleStats>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let Some(user) = auth.user else { return Ok(None) };

    let state = expect_context::<AppState>();
    let stats = state
        .puzzle_store
        .stats(user.id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(Some(stats))
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
