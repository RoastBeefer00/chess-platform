use leptos::prelude::*;
use shared::{RatingMode, TimeControl};
use uuid::Uuid;

#[server]
pub async fn search_for_game(
    time_control: TimeControl,
    rating_mode: RatingMode,
    player_id: Uuid,
) -> Result<Option<Uuid>, ServerFnError> {
    use crate::state::AppState;

    let state: AppState = expect_context();

    let rating = state
        .rating_store
        .get_rating(&player_id, time_control.category())
        .await?;

    let key = time_control.bucket(rating_mode);
    let window: u32 = 100;

    state
        .redis_client
        .add_to_bucket(&key, player_id, rating)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let opponent = state
        .redis_client
        .find_pair(&key, player_id, rating, window)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(opponent)
}
