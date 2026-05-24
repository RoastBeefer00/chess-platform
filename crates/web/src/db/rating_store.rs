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
}
