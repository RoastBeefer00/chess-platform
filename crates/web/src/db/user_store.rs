use shared::{Category, PlayerInfo};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::{AuthError, User};

#[derive(Clone, Debug)]
pub struct UserStore {
    pool: PgPool,
}

impl UserStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[tracing::instrument(skip(self))]
    pub async fn is_username_available(&self, username: String) -> Result<bool, AuthError> {
        Ok(
            sqlx::query_scalar!("SELECT 1 FROM users WHERE username = $1", username)
                .fetch_optional(&self.pool)
                .await?
                .is_none(),
        )
    }

    #[tracing::instrument(skip(self), fields(user_id = %user_id))]
    pub async fn set_username(&self, user_id: Uuid, username: String) -> Result<(), AuthError> {
        if self.is_username_available(username.clone()).await? {
            sqlx::query!(
                "UPDATE users SET username = $1 WHERE id = $2",
                username,
                user_id
            )
            .execute(&self.pool)
            .await?;
            Ok(())
        } else {
            Err(AuthError::UsernameTaken(username))
        }
    }

    #[tracing::instrument(skip(self), fields(user_id = %id, ?category))]
    pub async fn get_player_info(
        &self,
        id: &Uuid,
        category: Category,
    ) -> Result<PlayerInfo, AuthError> {
        let row = sqlx::query!(
            r#"SELECT u.id, u.username, u.avatar_url, r.rating
               FROM users u
               INNER JOIN ratings r ON r.user_id = u.id AND r.mode = $2
               WHERE u.id = $1"#,
            id,
            &category.to_string()
        )
        .fetch_one(&self.pool)
        .await?;

        Ok(PlayerInfo {
            id: row.id,
            username: row.username,
            avatar_url: row.avatar_url,
            rating: row.rating,
        })
    }

    #[tracing::instrument(skip(self), fields(user_id = %id))]
    pub async fn find_by_id(&self, id: &Uuid) -> Result<Option<User>, AuthError> {
        Ok(sqlx::query_as!(
            User,
            r#"SELECT id, email, username, avatar_url, bio, country, created_at
               FROM users WHERE id = $1"#,
            id
        )
        .fetch_optional(&self.pool)
        .await?)
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
    async fn username_available_for_fresh_user(pool: PgPool) {
        let store = UserStore::new(pool);
        let available = store.is_username_available("unused_name".to_string()).await.unwrap();
        assert!(available);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn set_username_succeeds_and_marks_taken(pool: PgPool) {
        let store = UserStore::new(pool.clone());
        let user_id = insert_user(&pool).await;
        store.set_username(user_id, "coolplayer".to_string()).await.unwrap();

        let still_available = store.is_username_available("coolplayer".to_string()).await.unwrap();
        assert!(!still_available, "username should be taken after set");

        let row = sqlx::query!("SELECT username FROM users WHERE id = $1", user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.username.as_deref(), Some("coolplayer"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn set_username_fails_on_duplicate(pool: PgPool) {
        let store = UserStore::new(pool.clone());
        let user_a = insert_user(&pool).await;
        let user_b = insert_user(&pool).await;
        store.set_username(user_a, "taken_name".to_string()).await.unwrap();

        let result = store.set_username(user_b, "taken_name".to_string()).await;
        assert!(
            matches!(result, Err(crate::auth::AuthError::UsernameTaken(_))),
            "expected UsernameTaken error"
        );
    }
}
