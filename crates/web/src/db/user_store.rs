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
