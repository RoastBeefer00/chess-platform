use leptos::prelude::*;
use shared::{AnalysisGameData, GameInfo};
use uuid::Uuid;

/// A finished (or aborted) game's move/clock history, for the analysis
/// board. Reads the DB directly rather than the in-memory `GameRoom` — by
/// the time this is reachable (the Analyze link only appears after a game
/// ends) the game is already persisted there.
#[server]
pub async fn get_game_for_analysis(game_id: Uuid) -> Result<AnalysisGameData, ServerFnError> {
    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let result = state
        .game_store
        .get_game_for_analysis(game_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("game not found or still in progress"));
    if let Err(ref e) = result {
        tracing::info!(%game_id, error = %e, "get_game_for_analysis failed");
    }
    result
}

#[server]
pub async fn get_game_info(game_id: Uuid) -> Result<GameInfo, ServerFnError> {
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::state::{AdoptOutcome, AppState};

    let state = expect_context::<AppState>();
    let game_room = match state.get_game_room(&game_id).await {
        Some(room) => room,
        // No local room — either a bogus id, or (this being a plain POST
        // with no `?game_id=`, so the fly-replay routing middleware in
        // `main.rs` never sees it) a game this instance simply doesn't own.
        // Adopting here covers the common case: the owning instance died
        // and this request is the first thing to touch the game since.
        None => match state.adopt_game(game_id).await {
            AdoptOutcome::Adopted(room) => room,
            AdoptOutcome::OwnedElsewhere | AdoptOutcome::Unadoptable => {
                return Err(ServerFnError::new("game not found"));
            }
        },
    };

    let (white_id, black_id, variant, white_ms_left, black_ms_left, clock_running, config) = {
        let gr = game_room.lock().await;
        (
            gr.game.white_player,
            gr.game.black_player,
            gr.game.config.time_control.category(),
            gr.game.white_ms_left,
            gr.game.black_ms_left,
            gr.last_move_at.is_some(),
            gr.game.config.clone(),
        )
    };

    let (white, black) = tokio::try_join!(
        state.user_store.get_player_info(&white_id, variant.clone()),
        state.user_store.get_player_info(&black_id, variant),
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    let sent_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    Ok(GameInfo {
        id: game_id,
        white,
        black,
        white_ms_left,
        black_ms_left,
        sent_at_ms,
        clock_running,
        config,
    })
}
