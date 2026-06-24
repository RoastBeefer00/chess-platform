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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;
    use uuid::Uuid;

    async fn insert_user(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, email) VALUES ($1, $2)",
            id,
            format!("{}@test.invalid", id)
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn no_history_returns_zero_diff(pool: PgPool) {
        let store = RatingStore::new(pool.clone());
        let user_id = insert_user(&pool).await;
        let (rating, diff) = store.get_rating_with_diff(&user_id, Category::Blitz).await.unwrap();
        assert_eq!(rating, 1500, "freshly created user starts at 1500");
        assert_eq!(diff, 0, "no history → diff is 0");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn history_older_than_14d_gives_diff(pool: PgPool) {
        let store = RatingStore::new(pool.clone());
        let user_id = insert_user(&pool).await;

        // Directly seed a rating_history row dated 15 days ago with rating 1400
        // to simulate a loss before the 14-day window.
        sqlx::query!(
            r#"INSERT INTO rating_history (user_id, mode, rating, recorded_at)
               VALUES ($1, 'blitz', 1400, now() - INTERVAL '15 days')"#,
            user_id
        )
        .execute(&pool)
        .await
        .unwrap();

        let (rating, diff) = store.get_rating_with_diff(&user_id, Category::Blitz).await.unwrap();
        assert_eq!(rating, 1500);
        // current (1500) - prior (1400) = +100
        assert_eq!(diff, 100, "14-day diff should reflect improvement from 1400 → 1500");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn recent_history_uses_oldest_in_window(pool: PgPool) {
        let store = RatingStore::new(pool.clone());
        let user_id = insert_user(&pool).await;

        // Seed a row within the 14-day window (10 days ago, rating 1480)
        // and a newer one (1 day ago, rating 1510). Oldest-in-window is 1480.
        sqlx::query!(
            "INSERT INTO rating_history (user_id, mode, rating, recorded_at) VALUES ($1, 'blitz', 1480, now() - INTERVAL '10 days')",
            user_id
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO rating_history (user_id, mode, rating, recorded_at) VALUES ($1, 'blitz', 1510, now() - INTERVAL '1 day')",
            user_id
        )
        .execute(&pool)
        .await
        .unwrap();

        let (rating, diff) = store.get_rating_with_diff(&user_id, Category::Blitz).await.unwrap();
        assert_eq!(rating, 1500);
        // current (1500) - oldest_in_window (1480) = +20
        assert_eq!(diff, 20);
    }
}
