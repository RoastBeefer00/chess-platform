use leptos::prelude::*;
use shared::GameInfo;
use uuid::Uuid;

#[server]
pub async fn get_game_info(game_id: Uuid) -> Result<GameInfo, ServerFnError> {
    use crate::state::AppState;
    use shared::GameMode;

    let state = expect_context::<AppState>();
    let game_room = state
        .get_game_room(&game_id)
        .await
        .ok_or_else(|| ServerFnError::new("game not found"))?;

    let (white_id, black_id) = {
        let gr = game_room.lock().await;
        (gr.game.white_player, gr.game.black_player)
    };

    // TODO: derive mode from the game itself once it's stored on Game.
    let mode = GameMode::Blitz;

    let (white, black) = tokio::try_join!(
        state.auth_backend.get_player_info(&white_id, mode.clone()),
        state.auth_backend.get_player_info(&black_id, mode),
    )
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(GameInfo {
        id: game_id,
        white,
        black,
    })
}
