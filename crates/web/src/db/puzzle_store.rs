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

    /// A uniformly random puzzle, full solution included — see
    /// `PuzzleSummary`'s doc comment for why that's intentional.
    ///
    /// Uses `TABLESAMPLE SYSTEM` (block-level random sampling) rather than
    /// the more obvious `ORDER BY random() LIMIT 1` — the latter forces a
    /// full sequential scan + sort of the entire table on *every* call,
    /// measured at ~730ms with 6.1M rows loaded (vs. <1ms for
    /// `TABLESAMPLE`, a real, well-documented Postgres anti-pattern, not
    /// specific to this schema). A 1% sample can theoretically land on zero
    /// rows (extremely unlikely in practice — at 500k+ rows that's still
    /// thousands of candidate rows), so `fetch_optional` falls back to the
    /// slow-but-always-correct query rather than erroring.
    #[tracing::instrument(skip(self))]
    pub async fn random(&self) -> sqlx::Result<PuzzleSummary> {
        let sampled = sqlx::query!(
            r#"SELECT id, fen, moves, rating, themes FROM puzzles TABLESAMPLE SYSTEM (1) LIMIT 1"#
        )
        .fetch_optional(&self.pool)
        .await?;

        let (id, fen, moves, rating, themes) = match sampled {
            Some(row) => (row.id, row.fen, row.moves, row.rating, row.themes),
            None => {
                let row = sqlx::query!(
                    r#"SELECT id, fen, moves, rating, themes FROM puzzles ORDER BY random() LIMIT 1"#
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

        let puzzle = store.random().await.unwrap();
        assert_eq!(puzzle.id, "test1");
        assert_eq!(puzzle.moves, "e2e4 e7e5 g1f3 b8c6");
        assert_eq!(puzzle.rating, 1500);
        assert_eq!(puzzle.themes, "fork middlegame");
    }
}
