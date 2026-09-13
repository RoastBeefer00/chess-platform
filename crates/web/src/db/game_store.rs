use shakmaty::KnownOutcome;
use shared::{
    messages::GameOverReason, AnalysisGameData, Category, GameStatus, RecentGame,
    RecentGamePlayer, RecentGameResult, Side,
};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::AuthError;
use crate::elo;

#[derive(Clone, Debug)]
pub struct GameStore {
    pool: PgPool,
}

/// Serializes per-move clocks as `"whiteMs,blackMs whiteMs,blackMs ..."`,
/// index-aligned with the `moves` column. `None` (→ SQL `NULL`) when there's
/// no clock history to persist (e.g. a game with no recorded moves).
fn clocks_to_string(clocks: &[(i64, i64)]) -> Option<String> {
    if clocks.is_empty() {
        return None;
    }
    Some(
        clocks
            .iter()
            .map(|(w, b)| format!("{w},{b}"))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Inverse of `clocks_to_string`. Tolerant of missing/malformed data — a
/// pair that doesn't parse becomes `None` rather than failing the whole
/// load, so older rows (predating clock tracking) still analyze, just
/// without clocks on the affected moves.
fn clocks_from_string(clocks: Option<&str>, move_count: usize) -> Vec<Option<(i64, i64)>> {
    let pairs: Vec<Option<(i64, i64)>> = clocks
        .unwrap_or_default()
        .split_whitespace()
        .map(|pair| {
            let (w, b) = pair.split_once(',')?;
            Some((w.parse().ok()?, b.parse().ok()?))
        })
        .collect();
    // Index-align with `moves` regardless of any count mismatch.
    (0..move_count)
        .map(|i| pairs.get(i).copied().flatten())
        .collect()
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
    /// (white_ms_left, black_ms_left) after each move in `moves`, same index
    /// alignment. Empty for games with no recorded clock history.
    pub clocks: Vec<(i64, i64)>,
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
        let clocks_joined: Option<String> = clocks_to_string(&plan.clocks);

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
                   clocks = $5,
                   final_fen = $6,
                   white_rating_before = $7,
                   black_rating_before = $8,
                   white_rating_after = $9,
                   black_rating_after = $10
               WHERE id = $1"#,
            plan.game_id,
            result_str,
            termination_str,
            moves_joined,
            clocks_joined,
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
        let clocks_joined = clocks_to_string(&plan.clocks);
        sqlx::query!(
            r#"UPDATE games
               SET status = 'aborted',
                   result = NULL,
                   termination = 'abandonment',
                   ended_at = now(),
                   moves = $2,
                   clocks = $3,
                   final_fen = $4
               WHERE id = $1"#,
            plan.game_id,
            moves_joined,
            clocks_joined,
            plan.final_fen,
        )
        .execute(&self.pool)
        .await?;
        tracing::info!(game_id = %plan.game_id, "game_aborted");
        Ok(())
    }

    /// Loads a game's move/clock history for the analysis board. `None` if
    /// the game doesn't exist or is still in progress — `moves`/`clocks`
    /// aren't written until `finalize_game`/`abort_game` runs.
    #[tracing::instrument(skip(self), fields(game_id = %game_id))]
    pub async fn get_game_for_analysis(
        &self,
        game_id: Uuid,
    ) -> Result<Option<AnalysisGameData>, AuthError> {
        let row = sqlx::query!(
            r#"SELECT moves, clocks, time_initial_seconds
               FROM games
               WHERE id = $1 AND status IN ('finished', 'aborted')"#,
            game_id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let moves: Vec<String> = r
                .moves
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_string)
                .collect();
            let clocks = clocks_from_string(r.clocks.as_deref(), moves.len());
            AnalysisGameData {
                moves,
                clocks,
                initial_time_ms: i64::from(r.time_initial_seconds) * 1000,
            }
        }))
    }

    /// Up to `limit` of `user_id`'s most recent finished/aborted games,
    /// newest first. Two indexed branches (`(white_user_id, ended_at)` /
    /// `(black_user_id, ended_at)`) unioned rather than a single
    /// `white_user_id = $1 OR black_user_id = $1`, which wouldn't use
    /// either index well.
    #[tracing::instrument(skip(self), fields(%user_id))]
    pub async fn list_recent_games(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RecentGame>, AuthError> {
        let rows = sqlx::query!(
            r#"WITH my_games AS (
                (SELECT id, white_user_id, black_user_id, result, status,
                        white_rating_after, black_rating_after, ended_at
                 FROM games
                 WHERE white_user_id = $1 AND status IN ('finished', 'aborted')
                 ORDER BY ended_at DESC LIMIT $2)
                UNION ALL
                (SELECT id, white_user_id, black_user_id, result, status,
                        white_rating_after, black_rating_after, ended_at
                 FROM games
                 WHERE black_user_id = $1 AND status IN ('finished', 'aborted')
                 ORDER BY ended_at DESC LIMIT $2)
            )
            SELECT mg.id AS "id!", mg.white_user_id AS "white_user_id!", mg.black_user_id AS "black_user_id!",
                   mg.result, mg.status AS "status!",
                   mg.white_rating_after, mg.black_rating_after,
                   wu.username AS white_username, wu.avatar_url AS white_avatar_url,
                   bu.username AS black_username, bu.avatar_url AS black_avatar_url
            FROM my_games mg
            JOIN users wu ON wu.id = mg.white_user_id
            JOIN users bu ON bu.id = mg.black_user_id
            ORDER BY mg.ended_at DESC
            LIMIT $2"#,
            user_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let my_result = if r.status == "aborted" {
                    RecentGameResult::Aborted
                } else {
                    match r.result.as_deref() {
                        Some("draw") => RecentGameResult::Drawn,
                        Some("white") if user_id == r.white_user_id => RecentGameResult::Won,
                        Some("white") => RecentGameResult::Lost,
                        Some("black") if user_id == r.black_user_id => RecentGameResult::Won,
                        Some("black") => RecentGameResult::Lost,
                        _ => RecentGameResult::Aborted,
                    }
                };
                RecentGame {
                    id: r.id,
                    white: RecentGamePlayer {
                        username: r.white_username,
                        avatar_url: r.white_avatar_url,
                        rating: r.white_rating_after,
                    },
                    black: RecentGamePlayer {
                        username: r.black_username,
                        avatar_url: r.black_avatar_url,
                        rating: r.black_rating_after,
                    },
                    my_result,
                    my_side: if user_id == r.white_user_id {
                        Side::White
                    } else {
                        Side::Black
                    },
                }
            })
            .collect())
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
            clocks: vec![(299_000, 300_000), (299_000, 298_500)],
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn get_game_for_analysis_returns_moves_and_clocks(pool: PgPool) {
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

        let data = store
            .get_game_for_analysis(game_id)
            .await
            .unwrap()
            .expect("finished game should be analyzable");
        assert_eq!(data.moves, vec!["e2e4".to_string(), "e7e5".to_string()]);
        assert_eq!(data.clocks, vec![Some((299_000, 300_000)), Some((299_000, 298_500))]);
        assert_eq!(data.initial_time_ms, 300_000);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn get_game_for_analysis_none_for_unfinished_game(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await; // still 'active'

        assert!(store.get_game_for_analysis(game_id).await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_recent_games_reports_result_relative_to_each_side(pool: PgPool) {
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

        let white_view = store.list_recent_games(white_id, 10).await.unwrap();
        assert_eq!(white_view.len(), 1);
        assert_eq!(white_view[0].my_result, RecentGameResult::Won);
        assert_eq!(white_view[0].my_side, Side::White);

        let black_view = store.list_recent_games(black_id, 10).await.unwrap();
        assert_eq!(black_view.len(), 1);
        assert_eq!(black_view[0].my_result, RecentGameResult::Lost);
        assert_eq!(black_view[0].my_side, Side::Black);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_recent_games_marks_aborted_with_no_rating(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await;
        let plan = make_plan(game_id, white_id, black_id, true, KnownOutcome::Draw, GameOverReason::Abort);
        store.abort_game(plan).await.unwrap();

        let rows = store.list_recent_games(white_id, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].my_result, RecentGameResult::Aborted);
        assert_eq!(rows[0].white.rating, None);
        assert_eq!(rows[0].black.rating, None);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_recent_games_respects_limit_and_recency(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        for _ in 0..3 {
            let game_id = Uuid::new_v4();
            insert_game_row(&pool, game_id, white_id, black_id, true).await;
            let plan = make_plan(
                game_id,
                white_id,
                black_id,
                true,
                KnownOutcome::Draw,
                GameOverReason::DrawAgreement,
            );
            store.finalize_game(plan).await.unwrap();
        }

        let rows = store.list_recent_games(white_id, 2).await.unwrap();
        assert_eq!(rows.len(), 2, "limit should cap the result count");
    }
}
