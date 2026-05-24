use leptos::prelude::*;
use shared::GameInfo;
use uuid::Uuid;

#[server]
pub async fn get_game_info(game_id: Uuid) -> Result<GameInfo, ServerFnError> {
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::state::AppState;

    let state = expect_context::<AppState>();
    let game_room = state
        .get_game_room(&game_id)
        .await
        .ok_or_else(|| ServerFnError::new("game not found"))?;

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
        state
            .auth_backend
            .get_player_info(&white_id, variant.clone()),
        state.auth_backend.get_player_info(&black_id, variant),
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
