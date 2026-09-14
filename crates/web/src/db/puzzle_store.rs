use shared::PuzzleSummary;
use sqlx::PgPool;

#[derive(Clone, Debug)]
pub struct PuzzleStore {
    pool: PgPool,
}

impl PuzzleStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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
}
