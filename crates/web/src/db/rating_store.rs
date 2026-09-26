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

    /// Every category's rating and 14-day diff for one user, in a single
    /// query.
    ///
    /// This replaced a per-category version: calling that once per
    /// `Category` meant four round trips on every home and profile page
    /// render, and because Leptos resolves those resources concurrently
    /// during SSR, four simultaneous connections — enough on its own to take
    /// most of the pool for a single page view.
    ///
    /// Both LATERAL subqueries correlate on `r.mode`, so each row gets its
    /// own category's history rather than one mode's diff applied to all
    /// four.
    ///
    /// The `ratings` table also holds '960' and 'puzzle' rows, which are real
    /// pools but not game categories; `Category::from_mode` returns `None`
    /// for those and they're skipped.
    #[tracing::instrument(skip(self), fields(user_id = %id))]
    pub async fn get_all_ratings_with_diff(
        &self,
        id: &Uuid,
    ) -> Result<Vec<(Category, u32, i32)>, AuthError> {
        let rows = sqlx::query!(
            r#"
            SELECT
                r.mode,
                r.rating AS current_rating,
                r.rating - COALESCE(prior.rating, oldest_in_window.rating, r.rating) AS diff
            FROM ratings r
            LEFT JOIN LATERAL (
                SELECT rating
                FROM rating_history
                WHERE user_id = r.user_id
                  AND mode = r.mode
                  AND recorded_at <= now() - INTERVAL '14 days'
                ORDER BY recorded_at DESC
                LIMIT 1
            ) prior ON true
            LEFT JOIN LATERAL (
                SELECT rating
                FROM rating_history
                WHERE user_id = r.user_id
                  AND mode = r.mode
                  AND recorded_at >= now() - INTERVAL '14 days'
                ORDER BY recorded_at ASC
                LIMIT 1
            ) oldest_in_window ON true
            WHERE r.user_id = $1
            "#,
            id
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|r| {
                Category::from_mode(&r.mode)
                    .map(|c| (c, r.current_rating as u32, r.diff.unwrap_or(0)))
            })
            .collect())
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

    /// Pulls one category out of the batched result.
    async fn one(store: &RatingStore, user_id: &Uuid, category: Category) -> (u32, i32) {
        store
            .get_all_ratings_with_diff(user_id)
            .await
            .unwrap()
            .into_iter()
            .find(|(c, _, _)| *c == category)
            .map(|(_, rating, diff)| (rating, diff))
            .unwrap_or_else(|| panic!("no {category:?} row"))
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn no_history_returns_zero_diff(pool: PgPool) {
        let store = RatingStore::new(pool.clone());
        let user_id = insert_user(&pool).await;
        let (rating, diff) = one(&store, &user_id, Category::Blitz).await;
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

        let (rating, diff) = one(&store, &user_id, Category::Blitz).await;
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

        let (rating, diff) = one(&store, &user_id, Category::Blitz).await;
        assert_eq!(rating, 1500);
        // current (1500) - oldest_in_window (1480) = +20
        assert_eq!(diff, 20);
    }

    /// The batched query's LATERAL joins correlate on `r.mode`. If they
    /// didn't, every category would report whichever mode's history the
    /// subquery happened to pick — so seed two modes and check they differ.
    #[sqlx::test(migrations = "../../migrations")]
    async fn diffs_do_not_bleed_between_categories(pool: PgPool) {
        let store = RatingStore::new(pool.clone());
        let user_id = insert_user(&pool).await;

        for (mode, rating) in [("blitz", 1400), ("rapid", 1600)] {
            sqlx::query!(
                r#"INSERT INTO rating_history (user_id, mode, rating, recorded_at)
                   VALUES ($1, $2, $3, now() - INTERVAL '15 days')"#,
                user_id,
                mode,
                rating
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        assert_eq!(one(&store, &user_id, Category::Blitz).await, (1500, 100));
        assert_eq!(one(&store, &user_id, Category::Rapid).await, (1500, -100));
        assert_eq!(
            one(&store, &user_id, Category::Bullet).await,
            (1500, 0),
            "a mode with no history keeps a zero diff"
        );
    }

    /// `ratings` also carries '960' and 'puzzle' rows, which aren't game
    /// categories. They must not appear in the result.
    #[sqlx::test(migrations = "../../migrations")]
    async fn non_category_modes_are_skipped(pool: PgPool) {
        let store = RatingStore::new(pool.clone());
        let user_id = insert_user(&pool).await;

        for mode in ["960", "puzzle"] {
            sqlx::query!(
                "INSERT INTO ratings (user_id, mode, rating) VALUES ($1, $2, 1500)
                 ON CONFLICT (user_id, mode) DO NOTHING",
                user_id,
                mode
            )
            .execute(&pool)
            .await
            .unwrap();
        }

        let categories: Vec<Category> = store
            .get_all_ratings_with_diff(&user_id)
            .await
            .unwrap()
            .into_iter()
            .map(|(c, _, _)| c)
            .collect();

        assert_eq!(categories.len(), 4, "exactly the four game categories: {categories:?}");
    }
}
