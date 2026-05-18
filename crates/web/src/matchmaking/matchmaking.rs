use leptos::prelude::*;
use shared::{Bucket, GameMode, GameStatus, RatingMode};
use uuid::Uuid;

#[server]
pub async fn search_for_game(
    bucket: Bucket,
    rating_mode: RatingMode,
    player_id: Uuid,
) -> Result<Option<Uuid>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let state: AppState = expect_context();

    let rating = auth
        .backend
        .get_user_rating(&player_id, bucket.mode())
        .await?;

    let key = bucket.id(rating_mode);
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
