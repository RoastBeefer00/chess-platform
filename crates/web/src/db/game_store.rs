use shakmaty::KnownOutcome;
use shared::{messages::GameOverReason, Category, GameStatus, Side};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::AuthError;
use crate::elo;

#[derive(Clone, Debug)]
pub struct GameStore {
    pool: PgPool,
}

/// Snapshot of everything `GameStore::finalize_game` needs to persist a
/// completed game. Built by `GameRoom::end_game` and shipped across the
/// async boundary to the finalize task.
#[derive(Debug, Clone)]
pub struct GameFinalization {
    pub game_id: Uuid,
    pub white_id: Uuid,
    pub black_id: Uuid,
    pub category: Category,
    pub rated: bool,
    pub moves: Vec<String>,
    pub final_fen: String,
    pub outcome: KnownOutcome,
    pub reason: GameOverReason,
}

impl GameStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(
        skip(self),
        fields(game_id = %id, white = %white_id, black = %black_id, ?category, rated)
    )]
    pub async fn insert_new_game(
        &self,
        id: &Uuid,
        status: &GameStatus,
        white_id: &Uuid,
        black_id: &Uuid,
        category: &Category,
        time_initial_seconds: i32,
        time_increment_seconds: i32,
        rated: bool,
    ) -> Result<(), AuthError> {
        let status_str = match status {
            GameStatus::WaitingForOpponent => "waiting",
            GameStatus::Ongoing => "active",
            GameStatus::Finished(_) => "finished",
        };
        let mode = category.to_string();
        sqlx::query!(
            r#"INSERT INTO games (
                id, status, white_user_id, black_user_id, mode,
                time_initial_seconds, time_increment_seconds, rated
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
            id,
            status_str,
            white_id,
            black_id,
            mode,
            time_initial_seconds,
            time_increment_seconds,
            rated,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Persist a completed game atomically: update `games`, recompute ELO,
    /// update both `ratings` rows, and append two `rating_history` rows. All
    /// in one transaction so a partial failure leaves no half-finalized state.
    /// Unrated games skip the rating writes but still persist the games row.
    #[tracing::instrument(
        skip(self, plan),
        fields(game_id = %plan.game_id, rated = plan.rated, ?plan.reason)
    )]
    pub async fn finalize_game(&self, plan: GameFinalization) -> Result<(), AuthError> {
        let mode = plan.category.to_string();
        let result_str = match plan.outcome {
            KnownOutcome::Decisive {
                winner: shakmaty::Color::White,
            } => "white",
            KnownOutcome::Decisive {
                winner: shakmaty::Color::Black,
            } => "black",
            KnownOutcome::Draw => "draw",
        };
        let termination_str = match plan.reason {
            GameOverReason::Checkmate => "checkmate",
            GameOverReason::Resignation => "resignation",
            GameOverReason::Timeout => "timeout",
            GameOverReason::Abort => "abandonment",
            GameOverReason::Stalemate => "stalemate",
            GameOverReason::InsufficientMaterial => "insufficient_material",
            GameOverReason::Repetition => "repetition",
            GameOverReason::FiftyMove => "fifty_move",
            GameOverReason::DrawAgreement => "draw_agreement",
        };
        let moves_joined = plan.moves.join(" ");

        let mut tx = self.pool.begin().await?;

        // Read current ratings + games count for both players up front.
        let (white_rating_before, white_games) = sqlx::query!(
            "SELECT rating, games FROM ratings WHERE user_id = $1 AND mode = $2",
            plan.white_id,
            mode,
        )
        .fetch_one(&mut *tx)
        .await
        .map(|r| (r.rating, r.games))?;

        let (black_rating_before, black_games) = sqlx::query!(
            "SELECT rating, games FROM ratings WHERE user_id = $1 AND mode = $2",
            plan.black_id,
            mode,
        )
        .fetch_one(&mut *tx)
        .await
        .map(|r| (r.rating, r.games))?;

        // Compute new ratings if the game is rated; otherwise carry the old
        // values through so the games row still records the snapshot.
        let (white_rating_after, black_rating_after) = if plan.rated {
            let (white_score, black_score) = match plan.outcome {
                KnownOutcome::Decisive {
                    winner: shakmaty::Color::White,
                } => (1.0, 0.0),
                KnownOutcome::Decisive {
                    winner: shakmaty::Color::Black,
                } => (0.0, 1.0),
                KnownOutcome::Draw => (0.5, 0.5),
            };
            let new_white = elo::new_rating(
                white_rating_before,
                black_rating_before,
                white_score,
                white_games,
            );
            let new_black = elo::new_rating(
                black_rating_before,
                white_rating_before,
                black_score,
                black_games,
            );
            (new_white, new_black)
        } else {
            (white_rating_before, black_rating_before)
        };

        sqlx::query!(
            r#"UPDATE games
               SET status = 'finished',
                   result = $2,
                   termination = $3,
                   ended_at = now(),
                   moves = $4,
                   final_fen = $5,
                   white_rating_before = $6,
                   black_rating_before = $7,
                   white_rating_after = $8,
                   black_rating_after = $9
               WHERE id = $1"#,
            plan.game_id,
            result_str,
            termination_str,
            moves_joined,
            plan.final_fen,
            white_rating_before,
            black_rating_before,
            white_rating_after,
            black_rating_after,
        )
        .execute(&mut *tx)
        .await?;

        if plan.rated {
            sqlx::query!(
                r#"UPDATE ratings
                   SET rating = $3, games = games + 1, updated_at = now()
                   WHERE user_id = $1 AND mode = $2"#,
                plan.white_id,
                mode,
                white_rating_after,
            )
            .execute(&mut *tx)
            .await?;

            sqlx::query!(
                r#"UPDATE ratings
                   SET rating = $3, games = games + 1, updated_at = now()
                   WHERE user_id = $1 AND mode = $2"#,
                plan.black_id,
                mode,
                black_rating_after,
            )
            .execute(&mut *tx)
            .await?;

            sqlx::query!(
                r#"INSERT INTO rating_history (user_id, mode, rating, game_id)
                   VALUES ($1, $2, $3, $4)"#,
                plan.white_id,
                mode,
                white_rating_after,
                plan.game_id,
            )
            .execute(&mut *tx)
            .await?;

            sqlx::query!(
                r#"INSERT INTO rating_history (user_id, mode, rating, game_id)
                   VALUES ($1, $2, $3, $4)"#,
                plan.black_id,
                mode,
                black_rating_after,
                plan.game_id,
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        let winner_side = match plan.outcome {
            KnownOutcome::Decisive { winner } => Some(Side::from(winner)),
            KnownOutcome::Draw => None,
        };
        tracing::info!(
            game_id = %plan.game_id,
            ?winner_side,
            white_delta = white_rating_after - white_rating_before,
            black_delta = black_rating_after - black_rating_before,
            "game_finalized"
        );
        Ok(())
    }

    /// Persist an aborted game: sets `status='aborted'`, `result=NULL`,
    /// `termination='abandonment'`. Skips all rating reads/writes — no ELO
    /// change for either player.
    #[tracing::instrument(skip(self, plan), fields(game_id = %plan.game_id))]
    pub async fn abort_game(&self, plan: GameFinalization) -> Result<(), AuthError> {
        let moves_joined = plan.moves.join(" ");
        sqlx::query!(
            r#"UPDATE games
               SET status = 'aborted',
                   result = NULL,
                   termination = 'abandonment',
                   ended_at = now(),
                   moves = $2,
                   final_fen = $3
               WHERE id = $1"#,
            plan.game_id,
            moves_joined,
            plan.final_fen,
        )
        .execute(&self.pool)
        .await?;
        tracing::info!(game_id = %plan.game_id, "game_aborted");
        Ok(())
    }
}

pub fn spawn_finalize(game_store: GameStore, plan: Option<GameFinalization>) {
    if let Some(plan) = plan {
        let gs = game_store.clone();
        tokio::spawn(async move {
            let result = if matches!(plan.reason, GameOverReason::Abort) {
                gs.abort_game(plan).await
            } else {
                gs.finalize_game(plan).await
            };
            if let Err(e) = result {
                tracing::warn!(?e, "game finalization failed");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{Category, GameConfig, GameStatus, RatingMode, TimeControl, TimeMode, Variant};
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

    async fn insert_game_row(pool: &PgPool, game_id: Uuid, white_id: Uuid, black_id: Uuid, rated: bool) {
        sqlx::query!(
            r#"INSERT INTO games (id, status, white_user_id, black_user_id, mode,
                time_initial_seconds, time_increment_seconds, rated)
               VALUES ($1, 'active', $2, $3, 'blitz', 300, 0, $4)"#,
            game_id,
            white_id,
            black_id,
            rated,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    fn make_plan(
        game_id: Uuid,
        white_id: Uuid,
        black_id: Uuid,
        rated: bool,
        outcome: KnownOutcome,
        reason: GameOverReason,
    ) -> GameFinalization {
        GameFinalization {
            game_id,
            white_id,
            black_id,
            category: Category::Blitz,
            rated,
            moves: vec!["e2e4".to_string(), "e7e5".to_string()],
            final_fen: "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2".to_string(),
            outcome,
            reason,
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn finalize_rated_white_wins_updates_ratings(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await;

        let plan = make_plan(
            game_id,
            white_id,
            black_id,
            true,
            KnownOutcome::Decisive { winner: shakmaty::Color::White },
            GameOverReason::Checkmate,
        );
        store.finalize_game(plan).await.unwrap();

        let white_rating: i32 = sqlx::query_scalar!(
            "SELECT rating FROM ratings WHERE user_id = $1 AND mode = 'blitz'",
            white_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let black_rating: i32 = sqlx::query_scalar!(
            "SELECT rating FROM ratings WHERE user_id = $1 AND mode = 'blitz'",
            black_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(white_rating > 1500, "winner's rating should increase (was {white_rating})");
        assert!(black_rating < 1500, "loser's rating should decrease (was {black_rating})");

        let history_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM rating_history WHERE game_id = $1",
            game_id
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(history_count, 2, "two rating_history rows (one per player)");

        let games_row = sqlx::query!(
            "SELECT status, result, termination FROM games WHERE id = $1",
            game_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(games_row.status, "finished");
        assert_eq!(games_row.result.as_deref(), Some("white"));
        assert_eq!(games_row.termination.as_deref(), Some("checkmate"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn finalize_rated_draw_gives_half_point(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await;

        let plan = make_plan(game_id, white_id, black_id, true, KnownOutcome::Draw, GameOverReason::Stalemate);
        store.finalize_game(plan).await.unwrap();

        let white_rating: i32 = sqlx::query_scalar!(
            "SELECT rating FROM ratings WHERE user_id = $1 AND mode = 'blitz'",
            white_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let black_rating: i32 = sqlx::query_scalar!(
            "SELECT rating FROM ratings WHERE user_id = $1 AND mode = 'blitz'",
            black_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // Equal-rating draw → no change (expected 0.5, got 0.5).
        assert_eq!(white_rating, 1500, "equal-rating draw: white unchanged");
        assert_eq!(black_rating, 1500, "equal-rating draw: black unchanged");

        let games_row = sqlx::query!("SELECT result FROM games WHERE id = $1", game_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(games_row.result.as_deref(), Some("draw"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn finalize_casual_game_does_not_change_ratings(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, false).await;

        let plan = make_plan(
            game_id,
            white_id,
            black_id,
            false,
            KnownOutcome::Decisive { winner: shakmaty::Color::White },
            GameOverReason::Checkmate,
        );
        store.finalize_game(plan).await.unwrap();

        let white_rating: i32 = sqlx::query_scalar!(
            "SELECT rating FROM ratings WHERE user_id = $1 AND mode = 'blitz'",
            white_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(white_rating, 1500, "casual: ratings must not change");

        let history_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM rating_history WHERE game_id = $1",
            game_id
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or(0);
        assert_eq!(history_count, 0, "casual: no rating_history rows");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn abort_game_sets_aborted_status(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await;

        let plan = make_plan(game_id, white_id, black_id, true, KnownOutcome::Draw, GameOverReason::Abort);
        store.abort_game(plan).await.unwrap();

        let row = sqlx::query!("SELECT status, result FROM games WHERE id = $1", game_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.status, "aborted");
        assert!(row.result.is_none(), "aborted game has no result");

        // No rating change.
        let white_rating: i32 = sqlx::query_scalar!(
            "SELECT rating FROM ratings WHERE user_id = $1 AND mode = 'blitz'",
            white_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(white_rating, 1500, "abort: ratings must not change");
    }
}
