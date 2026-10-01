use shared::{PuzzleStats, PuzzleSummary};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::AuthError;
use crate::elo;

#[derive(Clone, Debug)]
pub struct PuzzleStore {
    pool: PgPool,
}

impl PuzzleStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Records the outcome of `user_id`'s first attempt at `puzzle_id` and
    /// moves their puzzle rating accordingly, treating the puzzle's own
    /// rating as the opponent's.
    ///
    /// Idempotent per (user, puzzle): a repeat attempt is stored-once by the
    /// primary key and returns the unchanged stats, so replaying a puzzle
    /// you've already beaten can't farm rating. That also makes it safe for
    /// the client to call this without tracking whether it already has.
    ///
    /// Returns the stats as of after the attempt.
    #[tracing::instrument(skip(self), fields(%user_id, %puzzle_id, solved))]
    pub async fn record_attempt(
        &self,
        user_id: Uuid,
        puzzle_id: &str,
        solved: bool,
    ) -> Result<PuzzleStats, AuthError> {
        let mut tx = self.pool.begin().await?;

        let puzzle_rating: Option<i32> =
            sqlx::query_scalar!("SELECT rating FROM puzzles WHERE id = $1", puzzle_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(puzzle_rating) = puzzle_rating else {
            return Err(AuthError::Internal(format!("no such puzzle: {puzzle_id}")));
        };

        let current = sqlx::query!(
            "SELECT rating, games FROM ratings WHERE user_id = $1 AND mode = 'puzzle'",
            user_id,
        )
        .fetch_one(&mut *tx)
        .await?;

        let score = if solved { 1.0 } else { 0.0 };
        let new_rating = elo::new_rating(current.rating, puzzle_rating, score, current.games);

        // `ON CONFLICT DO NOTHING` is the idempotency gate — everything
        // below only runs when this actually inserted a row.
        let inserted = sqlx::query!(
            r#"INSERT INTO puzzle_attempts
                   (user_id, puzzle_id, solved, rating_before, rating_after)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT DO NOTHING
               RETURNING puzzle_id"#,
            user_id,
            puzzle_id,
            solved,
            current.rating,
            new_rating,
        )
        .fetch_optional(&mut *tx)
        .await?;

        if inserted.is_some() {
            sqlx::query!(
                r#"UPDATE ratings SET rating = $2, games = games + 1, updated_at = now()
                   WHERE user_id = $1 AND mode = 'puzzle'"#,
                user_id,
                new_rating,
            )
            .execute(&mut *tx)
            .await?;

            sqlx::query!(
                r#"INSERT INTO rating_history (user_id, mode, rating)
                   VALUES ($1, 'puzzle', $2)"#,
                user_id,
                new_rating,
            )
            .execute(&mut *tx)
            .await?;
        }

        let stats = Self::stats_tx(&mut tx, user_id).await?;
        tx.commit().await?;
        Ok(stats)
    }

    /// `user_id`'s puzzle rating and solve counts.
    #[tracing::instrument(skip(self), fields(%user_id))]
    pub async fn stats(&self, user_id: Uuid) -> Result<PuzzleStats, AuthError> {
        let mut tx = self.pool.begin().await?;
        let stats = Self::stats_tx(&mut tx, user_id).await?;
        tx.commit().await?;
        Ok(stats)
    }

    async fn stats_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        user_id: Uuid,
    ) -> Result<PuzzleStats, AuthError> {
        let rating: i32 = sqlx::query_scalar!(
            "SELECT rating FROM ratings WHERE user_id = $1 AND mode = 'puzzle'",
            user_id,
        )
        .fetch_one(&mut **tx)
        .await?;

        let counts = sqlx::query!(
            r#"SELECT
                   COUNT(*) AS "attempted!",
                   COUNT(*) FILTER (WHERE solved) AS "solved!"
               FROM puzzle_attempts WHERE user_id = $1"#,
            user_id,
        )
        .fetch_one(&mut **tx)
        .await?;

        Ok(PuzzleStats {
            rating,
            solved: counts.solved.max(0) as u32,
            attempted: counts.attempted.max(0) as u32,
        })
    }

    /// A uniformly random puzzle matching `themes` (any-of; empty = no
    /// theme filter) and within `[min_rating, max_rating]`, full solution
    /// included — see `PuzzleSummary`'s doc comment for why that's
    /// intentional.
    ///
    /// Uses `TABLESAMPLE SYSTEM` (block-level random sampling) rather than
    /// the more obvious `ORDER BY random() LIMIT 1` — the latter forces a
    /// full sequential scan + sort of the entire table on *every* call,
    /// measured at ~730ms with 6.1M rows loaded (vs. <1ms for
    /// `TABLESAMPLE`, a real, well-documented Postgres anti-pattern, not
    /// specific to this schema). A 1% sample can land on zero *matching*
    /// rows more easily once a `WHERE` clause is added — a rare theme, or a
    /// narrow rating band, might have no representative in that particular
    /// 1% of pages even though matches exist elsewhere — so `fetch_optional`
    /// falls back to the slow-but-always-correct filtered full scan rather
    /// than erroring or returning nothing.
    #[tracing::instrument(skip(self, themes))]
    pub async fn random(
        &self,
        themes: &[String],
        min_rating: i32,
        max_rating: i32,
    ) -> sqlx::Result<PuzzleSummary> {
        let sampled = sqlx::query!(
            r#"SELECT id, fen, moves, rating, themes FROM puzzles
               TABLESAMPLE SYSTEM (1)
               WHERE rating BETWEEN $1 AND $2
                 AND ($3::text[] = '{}' OR string_to_array(themes, ' ') && $3::text[])
               LIMIT 1"#,
            min_rating,
            max_rating,
            themes,
        )
        .fetch_optional(&self.pool)
        .await?;

        let (id, fen, moves, rating, themes) = match sampled {
            Some(row) => (row.id, row.fen, row.moves, row.rating, row.themes),
            None => {
                let row = sqlx::query!(
                    r#"SELECT id, fen, moves, rating, themes FROM puzzles
                       WHERE rating BETWEEN $1 AND $2
                         AND ($3::text[] = '{}' OR string_to_array(themes, ' ') && $3::text[])
                       ORDER BY random() LIMIT 1"#,
                    min_rating,
                    max_rating,
                    themes,
                )
                .fetch_one(&self.pool)
                .await?;
                (row.id, row.fen, row.moves, row.rating, row.themes)
            }
        };

        Ok(PuzzleSummary { id, fen, moves, rating, themes })
    }

    /// A specific puzzle by its (lichess-derived) id — the primary key, so
    /// this is a trivial indexed lookup. Used to resolve a shared puzzle
    /// link (`/puzzles?id=...`). `None` for an unknown id.
    #[tracing::instrument(skip(self))]
    pub async fn by_id(&self, id: &str) -> sqlx::Result<Option<PuzzleSummary>> {
        let row = sqlx::query!(
            "SELECT id, fen, moves, rating, themes FROM puzzles WHERE id = $1",
            id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| PuzzleSummary {
            id: r.id,
            fen: r.fen,
            moves: r.moves,
            rating: r.rating,
            themes: r.themes,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn insert_puzzle(pool: &PgPool, id: &str, moves: &str) {
        sqlx::query!(
            r#"INSERT INTO puzzles (id, fen, moves, rating, rating_deviation, popularity, nb_plays, themes)
               VALUES ($1, 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1', $2, 1500, 75, 90, 1000, 'fork middlegame')"#,
            id,
            moves,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn random_returns_a_puzzle_with_full_solution(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        insert_puzzle(&pool, "test1", "e2e4 e7e5 g1f3 b8c6").await;

        let puzzle = store.random(&[], 0, 4000).await.unwrap();
        assert_eq!(puzzle.id, "test1");
        assert_eq!(puzzle.moves, "e2e4 e7e5 g1f3 b8c6");
        assert_eq!(puzzle.rating, 1500);
        assert_eq!(puzzle.themes, "fork middlegame");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn random_filters_by_theme_and_rating(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        sqlx::query!(
            r#"INSERT INTO puzzles (id, fen, moves, rating, rating_deviation, popularity, nb_plays, themes)
               VALUES ('testA', 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1', 'e2e4 e7e5', 1200, 75, 90, 1000, 'fork middlegame')"#
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            r#"INSERT INTO puzzles (id, fen, moves, rating, rating_deviation, popularity, nb_plays, themes)
               VALUES ('testB', 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1', 'd2d4 d7d5', 2400, 75, 90, 1000, 'endgame rookEndgame')"#
        )
        .execute(&pool)
        .await
        .unwrap();

        // Theme filter narrows to the matching puzzle regardless of rating range.
        for _ in 0..5 {
            let puzzle = store.random(&["fork".to_string()], 0, 4000).await.unwrap();
            assert_eq!(puzzle.id, "testA");
        }

        // Rating filter alone also narrows correctly.
        for _ in 0..5 {
            let puzzle = store.random(&[], 2000, 4000).await.unwrap();
            assert_eq!(puzzle.id, "testB");
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn by_id_returns_matching_puzzle(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        insert_puzzle(&pool, "00sHx", "e2e4 e7e5 g1f3 b8c6").await;

        let puzzle = store.by_id("00sHx").await.unwrap().expect("puzzle should exist");
        assert_eq!(puzzle.moves, "e2e4 e7e5 g1f3 b8c6");
        assert_eq!(puzzle.rating, 1500);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn by_id_none_for_unknown_id(pool: PgPool) {
        let store = PuzzleStore::new(pool);
        assert!(store.by_id("does-not-exist").await.unwrap().is_none());
    }

    // ── puzzle progress ──────────────────────────────────────────────────

    async fn insert_user(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, email) VALUES ($1, $2)",
            id,
            format!("{id}@test.invalid"),
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn fresh_user_starts_at_the_default_puzzle_rating(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        let user_id = insert_user(&pool).await;

        let stats = store.stats(user_id).await.unwrap();
        assert_eq!(stats, shared::PuzzleStats { rating: 1500, solved: 0, attempted: 0 });
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn solving_raises_the_rating_and_counts_the_solve(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        let user_id = insert_user(&pool).await;
        insert_puzzle(&pool, "p1", "e2e4 e7e5").await;

        let stats = store.record_attempt(user_id, "p1", true).await.unwrap();
        assert!(stats.rating > 1500, "a solve at equal rating should gain; got {}", stats.rating);
        assert_eq!(stats.solved, 1);
        assert_eq!(stats.attempted, 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn failing_lowers_the_rating_but_still_counts_as_attempted(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        let user_id = insert_user(&pool).await;
        insert_puzzle(&pool, "p1", "e2e4 e7e5").await;

        let stats = store.record_attempt(user_id, "p1", false).await.unwrap();
        assert!(stats.rating < 1500, "a failure at equal rating should lose; got {}", stats.rating);
        assert_eq!(stats.solved, 0);
        assert_eq!(stats.attempted, 1);
    }

    /// The whole point of the (user, puzzle) primary key: replaying a puzzle
    /// you have already beaten must not move your rating again.
    #[sqlx::test(migrations = "../../migrations")]
    async fn re_attempting_the_same_puzzle_is_a_no_op(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        let user_id = insert_user(&pool).await;
        insert_puzzle(&pool, "p1", "e2e4 e7e5").await;

        let first = store.record_attempt(user_id, "p1", true).await.unwrap();
        let second = store.record_attempt(user_id, "p1", true).await.unwrap();
        assert_eq!(first, second, "a repeat attempt must not change anything");

        // Nor does a later *failure* overwrite a recorded success — first
        // outcome wins, which is what lets the client report from three
        // different places without coordinating.
        let third = store.record_attempt(user_id, "p1", false).await.unwrap();
        assert_eq!(first, third);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn distinct_puzzles_accumulate(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        let user_id = insert_user(&pool).await;
        insert_puzzle(&pool, "p1", "e2e4 e7e5").await;
        insert_puzzle(&pool, "p2", "d2d4 d7d5").await;

        store.record_attempt(user_id, "p1", true).await.unwrap();
        let stats = store.record_attempt(user_id, "p2", false).await.unwrap();
        assert_eq!(stats.solved, 1);
        assert_eq!(stats.attempted, 2);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn unknown_puzzle_is_an_error(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        let user_id = insert_user(&pool).await;
        assert!(store.record_attempt(user_id, "nope", true).await.is_err());
    }
}
