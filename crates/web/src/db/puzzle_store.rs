use shared::{MoveCheck, PuzzleSummary};
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
    /// theme filter) and within `[min_rating, max_rating]`. Only
    /// `moves[0]` (the opponent's setup move, already visible on the board
    /// once played — see `PuzzleSummary::first_move`) is exposed; the rest
    /// of the solution never leaves the server until checked move by move
    /// via `check_move`.
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

        let first_move = moves.split_whitespace().next().unwrap_or_default().to_string();

        Ok(PuzzleSummary { id, fen, first_move, rating, themes })
    }

    /// Checks the solver's move at `ply` (0-indexed among the solver's own
    /// moves) against the stored solution. `moves[0]` is the opponent's
    /// setup move (auto-played before the solver acts, not part of `ply`
    /// counting), so the solver's `ply`-th move lives at `moves[1 + 2*ply]`
    /// and, if present, the opponent's forced reply follows immediately
    /// after. A puzzle whose solution is odd-length (lichess's own
    /// convention: always ends on a solver move) is solved once that final
    /// index is reached correctly, with no reply to auto-play.
    #[tracing::instrument(skip(self, uci), fields(%id, ply))]
    pub async fn check_move(&self, id: &str, ply: usize, uci: &str) -> sqlx::Result<MoveCheck> {
        let Some(row) = sqlx::query!("SELECT moves FROM puzzles WHERE id = $1", id)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(MoveCheck::Incorrect);
        };

        let moves: Vec<&str> = row.moves.split_whitespace().collect();
        let idx = 1 + 2 * ply;

        if moves.get(idx) != Some(&uci) {
            return Ok(MoveCheck::Incorrect);
        }

        if idx == moves.len() - 1 {
            return Ok(MoveCheck::Correct { reply: None, solved: true });
        }

        Ok(MoveCheck::Correct {
            reply: moves.get(idx + 1).map(|s| s.to_string()),
            solved: false,
        })
    }

    /// The correct move (UCI) at `ply`, for the "reveal the move" hint
    /// stage — deliberately a separate, explicitly-requested call rather
    /// than part of `random`/`check_move`'s own responses, so the solution
    /// still never reaches the client unless asked for.
    #[tracing::instrument(skip(self), fields(%id, ply))]
    pub async fn hint(&self, id: &str, ply: usize) -> sqlx::Result<Option<String>> {
        let Some(row) = sqlx::query!("SELECT moves FROM puzzles WHERE id = $1", id)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(None);
        };

        let idx = 1 + 2 * ply;
        Ok(row.moves.split_whitespace().nth(idx).map(str::to_string))
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
    async fn random_returns_a_puzzle_without_the_solution(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        insert_puzzle(&pool, "test1", "e2e4 e7e5 g1f3 b8c6").await;

        let puzzle = store.random(&[], 0, 4000).await.unwrap();
        assert_eq!(puzzle.id, "test1");
        assert_eq!(puzzle.first_move, "e2e4");
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
    async fn check_move_correct_mid_puzzle_returns_reply(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        // setup, solver(0), reply, solver(1) — 4 moves total.
        insert_puzzle(&pool, "test2", "e2e4 e7e5 g1f3 b8c6").await;

        let result = store.check_move("test2", 0, "e7e5").await.unwrap();
        assert_eq!(
            result,
            MoveCheck::Correct { reply: Some("g1f3".to_string()), solved: false }
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn check_move_correct_final_move_solves_puzzle(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        insert_puzzle(&pool, "test3", "e2e4 e7e5 g1f3 b8c6").await;

        let result = store.check_move("test3", 1, "b8c6").await.unwrap();
        assert_eq!(result, MoveCheck::Correct { reply: None, solved: true });
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn check_move_wrong_move_is_incorrect(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        insert_puzzle(&pool, "test4", "e2e4 e7e5 g1f3 b8c6").await;

        let result = store.check_move("test4", 0, "d7d5").await.unwrap();
        assert_eq!(result, MoveCheck::Incorrect);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn check_move_unknown_puzzle_is_incorrect(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        let result = store.check_move("nonexistent", 0, "e7e5").await.unwrap();
        assert_eq!(result, MoveCheck::Incorrect);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn hint_returns_the_correct_move_at_ply(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        insert_puzzle(&pool, "test5", "e2e4 e7e5 g1f3 b8c6").await;

        assert_eq!(store.hint("test5", 0).await.unwrap(), Some("e7e5".to_string()));
        assert_eq!(store.hint("test5", 1).await.unwrap(), Some("b8c6".to_string()));
        assert_eq!(store.hint("test5", 2).await.unwrap(), None);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn hint_unknown_puzzle_returns_none(pool: PgPool) {
        let store = PuzzleStore::new(pool.clone());
        assert_eq!(store.hint("nonexistent", 0).await.unwrap(), None);
    }
}
