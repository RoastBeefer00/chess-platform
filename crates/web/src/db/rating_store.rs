use shared::Category;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::AuthError;

#[derive(Clone, Debug)]
pub struct RatingStore {
    pool: PgPool,
}

impl RatingStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[tracing::instrument(skip(self), fields(user_id = %id, ?category))]
    pub async fn get_rating(&self, id: &Uuid, category: Category) -> Result<u32, AuthError> {
        let rating = sqlx::query_scalar!(
            "SELECT rating FROM ratings WHERE user_id = $1 AND mode = $2",
            id,
            &category.to_string()
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(rating as u32)
    }

    #[tracing::instrument(skip(self), fields(user_id = %id, ?category))]
    pub async fn get_rating_with_diff(
        &self,
        id: &Uuid,
        category: Category,
    ) -> Result<(u32, i32), AuthError> {
        let row = sqlx::query!(
            r#"
            SELECT
                r.rating AS current_rating,
                r.rating - COALESCE(prior.rating, oldest_in_window.rating, r.rating) AS diff
            FROM ratings r
            LEFT JOIN LATERAL (
                SELECT rating
                FROM rating_history
                WHERE user_id = $1
                  AND mode = $2
                  AND recorded_at <= now() - INTERVAL '14 days'
                ORDER BY recorded_at DESC
                LIMIT 1
            ) prior ON true
            LEFT JOIN LATERAL (
                SELECT rating
                FROM rating_history
                WHERE user_id = $1
                  AND mode = $2
                  AND recorded_at >= now() - INTERVAL '14 days'
                ORDER BY recorded_at ASC
                LIMIT 1
            ) oldest_in_window ON true
            WHERE r.user_id = $1 AND r.mode = $2
            "#,
            id,
            &category.to_string()
        )
        .fetch_one(&self.pool)
        .await?;

        Ok((
            row.current_rating as u32,
            row.diff.unwrap_or(0),
        ))
    }
}
