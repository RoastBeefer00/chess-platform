use shakmaty::KnownOutcome;
use shared::{
    messages::GameOverReason, ActiveGame, AnalysisGameData, Category, FriendActiveGame, GameStatus,
    RecentGame, RecentGamePlayer, RecentGameResult, Side,
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

/// Strict counterpart to `clocks_from_string`, for game adoption — a
/// malformed or short clock string must fail the whole load rather than
/// silently produce wrong clocks (unlike analysis, where a `None` entry on
/// an old pre-clock-tracking row is an acceptable, cosmetic gap).
fn clocks_from_string_strict(clocks: Option<&str>, move_count: usize) -> Result<Vec<(i64, i64)>, String> {
    let pairs: Vec<(i64, i64)> = clocks
        .unwrap_or_default()
        .split_whitespace()
        .map(|pair| {
            let (w, b) = pair.split_once(',').ok_or_else(|| format!("malformed clock pair: {pair:?}"))?;
            Ok((
                w.parse::<i64>().map_err(|_| format!("bad white ms: {w:?}"))?,
                b.parse::<i64>().map_err(|_| format!("bad black ms: {b:?}"))?,
            ))
        })
        .collect::<Result<_, String>>()?;
    if pairs.len() != move_count {
        return Err(format!(
            "clock count {} doesn't match move count {move_count}",
            pairs.len()
        ));
    }
    Ok(pairs)
}

/// Everything needed to rebuild a `GameRoom` for a still-`active` game — see
/// `AppState::adopt_game`.
#[derive(Debug)]
pub struct ActiveGameRow {
    pub white_user_id: Uuid,
    pub black_user_id: Uuid,
    pub rated: bool,
    pub time_initial_seconds: i32,
    pub time_increment_seconds: i32,
    pub moves: Vec<String>,
    pub clocks: Vec<(i64, i64)>,
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

    /// All game ids currently `active`, for the heartbeat-aware reaper (see
    /// `AppState::reconcile_stale_active_games`) to check each one's
    /// `active_games:{id}` Redis heartbeat against.
    #[tracing::instrument(skip(self))]
    pub async fn list_active_game_ids(&self) -> Result<Vec<Uuid>, AuthError> {
        let rows = sqlx::query_scalar!("SELECT id FROM games WHERE status = 'active'")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }

    /// Marks exactly the given ids `aborted` — used by the reaper once it's
    /// confirmed each one's owning instance has actually gone quiet (Redis
    /// heartbeat lapsed), not just "some row is active". Same field set as
    /// `abort_game`: leaves `moves`/`clocks`/`final_fen` untouched, since
    /// real history already exists on these rows via incremental
    /// persistence (`persist_progress`) — only flips
    /// status/result/termination/ended_at. `WHERE status = 'active'` guards
    /// against a TOCTOU race where the game legitimately finished between
    /// the reaper's heartbeat check and this write.
    #[tracing::instrument(skip(self, ids), fields(n = ids.len()))]
    pub async fn abort_stale_games(&self, ids: &[Uuid]) -> Result<u64, AuthError> {
        if ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query!(
            r#"UPDATE games
               SET status = 'aborted',
                   result = NULL,
                   termination = 'abandonment',
                   ended_at = now()
               WHERE id = ANY($1) AND status = 'active'"#,
            ids,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// A game's bare `status` column, for a reconnect attempt that finds no
    /// live `GameRoom` (e.g. the reaper aborted it after its owning
    /// instance went quiet) to distinguish "this game ended" from "this id
    /// never existed" — see the `game_websocket` join handler.
    #[tracing::instrument(skip(self), fields(%game_id))]
    pub async fn get_game_status(&self, game_id: Uuid) -> Result<Option<String>, AuthError> {
        Ok(sqlx::query_scalar!("SELECT status FROM games WHERE id = $1", game_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Persists move/clock history for a still-in-progress game. Called
    /// after every move (fire-and-forget, see `spawn_progress_persist`) so a
    /// game killed mid-flight — server restart, Fly autostop — still has its
    /// full history on disk instead of losing everything, since otherwise
    /// `moves`/`clocks` stay `NULL` until `finalize_game`/`abort_game` runs
    /// at game end.
    ///
    /// `WHERE status = 'active'` is deliberate: once `finalize_game`/
    /// `abort_game` flips status away from `'active'`, a late in-flight
    /// progress write becomes a no-op (0 rows) instead of racing/clobbering
    /// the authoritative final write — no ordering or locking needed between
    /// this and the end-of-game path.
    #[tracing::instrument(skip(self, moves, clocks), fields(game_id = %game_id))]
    pub async fn persist_progress(
        &self,
        game_id: Uuid,
        moves: &[String],
        clocks: &[(i64, i64)],
    ) -> Result<(), AuthError> {
        let moves_joined = moves.join(" ");
        let clocks_joined = clocks_to_string(clocks);
        sqlx::query!(
            r#"UPDATE games SET moves = $2, clocks = $3 WHERE id = $1 AND status = 'active'"#,
            game_id,
            moves_joined,
            clocks_joined,
        )
        .execute(&self.pool)
        .await?;
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

    /// Loads a still-`active` game's move/clock history and time control,
    /// for the adoption path (`AppState::adopt_game`) to rebuild a
    /// `GameRoom` on an instance that didn't create it. `None` if the row
    /// doesn't exist or isn't `active` (already finished/aborted, or a
    /// bogus id). `Err` only on a genuinely malformed `clocks` column
    /// (count mismatch or unparseable pair) — the caller treats that as
    /// unadoptable, same as a replay failure.
    #[tracing::instrument(skip(self), fields(game_id = %game_id))]
    pub async fn load_active_game(&self, game_id: Uuid) -> Result<Option<ActiveGameRow>, AuthError> {
        let row = sqlx::query!(
            r#"SELECT white_user_id, black_user_id, rated,
                      time_initial_seconds, time_increment_seconds, moves, clocks
               FROM games
               WHERE id = $1 AND status = 'active'"#,
            game_id
        )
        .fetch_optional(&self.pool)
        .await?;

        let Some(r) = row else { return Ok(None) };
        let moves: Vec<String> = r
            .moves
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let clocks = clocks_from_string_strict(r.clocks.as_deref(), moves.len())
            .map_err(|e| AuthError::Internal(format!("game {game_id}: {e}")))?;

        Ok(Some(ActiveGameRow {
            white_user_id: r.white_user_id,
            black_user_id: r.black_user_id,
            rated: r.rated,
            time_initial_seconds: r.time_initial_seconds,
            time_increment_seconds: r.time_increment_seconds,
            moves,
            clocks,
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
                        white_rating_after, black_rating_after, ended_at, rated
                 FROM games
                 WHERE white_user_id = $1 AND status IN ('finished', 'aborted')
                 ORDER BY ended_at DESC LIMIT $2)
                UNION ALL
                (SELECT id, white_user_id, black_user_id, result, status,
                        white_rating_after, black_rating_after, ended_at, rated
                 FROM games
                 WHERE black_user_id = $1 AND status IN ('finished', 'aborted')
                 ORDER BY ended_at DESC LIMIT $2)
            )
            SELECT mg.id AS "id!", mg.white_user_id AS "white_user_id!", mg.black_user_id AS "black_user_id!",
                   mg.result, mg.status AS "status!", mg.rated AS "rated!",
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
                    rated: r.rated,
                }
            })
            .collect())
    }

    /// `user_id`'s currently in-progress game, if any. A user is only ever
    /// `status = 'active'` in one game at a time, so `LIMIT 1` on a
    /// `UNION ALL` of the two indexed branches is enough — no `ORDER BY`
    /// needed since at most one row can come back.
    #[tracing::instrument(skip(self), fields(%user_id))]
    pub async fn find_active_game(&self, user_id: Uuid) -> Result<Option<ActiveGame>, AuthError> {
        let row = sqlx::query!(
            r#"WITH my_game AS (
                (SELECT id, white_user_id, black_user_id FROM games
                 WHERE white_user_id = $1 AND status = 'active' LIMIT 1)
                UNION ALL
                (SELECT id, white_user_id, black_user_id FROM games
                 WHERE black_user_id = $1 AND status = 'active' LIMIT 1)
            )
            SELECT mg.id AS "id!", mg.white_user_id AS "white_user_id!", mg.black_user_id AS "black_user_id!",
                   wu.username AS white_username, wu.avatar_url AS white_avatar_url,
                   bu.username AS black_username, bu.avatar_url AS black_avatar_url
            FROM my_game mg
            JOIN users wu ON wu.id = mg.white_user_id
            JOIN users bu ON bu.id = mg.black_user_id
            LIMIT 1"#,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let (my_side, opponent) = if user_id == r.white_user_id {
                (
                    Side::White,
                    RecentGamePlayer {
                        username: r.black_username,
                        avatar_url: r.black_avatar_url,
                        rating: None,
                    },
                )
            } else {
                (
                    Side::Black,
                    RecentGamePlayer {
                        username: r.white_username,
                        avatar_url: r.white_avatar_url,
                        rating: None,
                    },
                )
            };
            ActiveGame {
                id: r.id,
                opponent,
                my_side,
            }
        }))
    }

    /// Which of `user_ids` are currently in an active game, and against whom.
    /// One scan over the small `status = 'active'` partial index rather than
    /// N calls to `find_active_game` — the friends-list "in game" column
    /// needs this for a whole list at once. Keyed by *participant* user_id,
    /// so a game between two friends appears twice, once under each side.
    #[tracing::instrument(skip(self, user_ids), fields(n = user_ids.len()))]
    pub async fn active_games_for(
        &self,
        user_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, FriendActiveGame>, AuthError> {
        if user_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let rows = sqlx::query!(
            r#"SELECT g.id, g.white_user_id, g.black_user_id,
                      wu.username AS white_username, bu.username AS black_username
               FROM games g
               JOIN users wu ON wu.id = g.white_user_id
               JOIN users bu ON bu.id = g.black_user_id
               WHERE g.status = 'active' AND (g.white_user_id = ANY($1) OR g.black_user_id = ANY($1))"#,
            user_ids,
        )
        .fetch_all(&self.pool)
        .await?;

        let wanted: std::collections::HashSet<Uuid> = user_ids.iter().copied().collect();
        let mut result = std::collections::HashMap::new();
        for r in rows {
            if wanted.contains(&r.white_user_id) {
                result.insert(
                    r.white_user_id,
                    FriendActiveGame { game_id: r.id, opponent_username: r.black_username.clone() },
                );
            }
            if wanted.contains(&r.black_user_id) {
                result.insert(
                    r.black_user_id,
                    FriendActiveGame { game_id: r.id, opponent_username: r.white_username.clone() },
                );
            }
        }
        Ok(result)
    }
}

/// Fire-and-forget per-move persistence — see `GameStore::persist_progress`.
/// Never awaited by the move path; a slow or failed write here must not add
/// latency to a move reaching the opponent, which stays instant via the
/// in-memory broadcast channel regardless of how this turns out.
pub fn spawn_progress_persist(game_store: GameStore, game_id: Uuid, moves: Vec<String>, clocks: Vec<(i64, i64)>) {
    tokio::spawn(async move {
        if let Err(e) = game_store.persist_progress(game_id, &moves, &clocks).await {
            tracing::warn!(?e, %game_id, "progress persist failed");
        }
    });
}

/// Persists a just-ended game's result and clears its cross-instance
/// watch-grid/ownership entry (see `AppState::create_game` and
/// `RedisClient::active_game_upsert` for where that entry starts). A no-op
/// for `None` (the game was already finished — defensive, shouldn't
/// normally happen on the calling code's paths).
///
/// Deliberately awaited directly rather than spawned detached — the caller
/// should `drop` any `GameRoom` lock guard it's still holding first (this
/// does one Postgres transaction, no need to hold the room locked for it).
/// A detached task takes an extra scheduling hop to even begin running,
/// which is exactly what turned a game decided by timeout into a silently
/// lost result: the process was killed before the detached finalize task
/// was ever polled. See main.rs's shutdown grace period for the other half
/// of this fix — that protects an in-flight write regardless of whether
/// it's spawned or awaited, but an already-running write has a real head
/// start on one that hasn't been scheduled yet.
pub async fn finalize_now(
    game_store: &GameStore,
    redis_client: &crate::state::RedisClient,
    plan: Option<GameFinalization>,
) {
    let Some(plan) = plan else { return };
    let game_id = plan.game_id;
    let result = if matches!(plan.reason, GameOverReason::Abort) {
        game_store.abort_game(plan).await
    } else {
        game_store.finalize_game(plan).await
    };
    if let Err(e) = result {
        tracing::warn!(?e, "game finalization failed");
    }
    redis_client.active_game_remove(game_id).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::Category;
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_recent_games_reports_rated_flag(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;

        let rated_game_id = Uuid::new_v4();
        insert_game_row(&pool, rated_game_id, white_id, black_id, true).await;
        let rated_plan = make_plan(
            rated_game_id,
            white_id,
            black_id,
            true,
            KnownOutcome::Decisive { winner: shakmaty::Color::White },
            GameOverReason::Checkmate,
        );
        store.finalize_game(rated_plan).await.unwrap();

        let casual_game_id = Uuid::new_v4();
        insert_game_row(&pool, casual_game_id, white_id, black_id, false).await;
        let casual_plan = make_plan(
            casual_game_id,
            white_id,
            black_id,
            false,
            KnownOutcome::Decisive { winner: shakmaty::Color::White },
            GameOverReason::Checkmate,
        );
        store.finalize_game(casual_plan).await.unwrap();

        let rows = store.list_recent_games(white_id, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        let rated_row = rows.iter().find(|r| r.id == rated_game_id).unwrap();
        let casual_row = rows.iter().find(|r| r.id == casual_game_id).unwrap();
        assert!(rated_row.rated);
        assert!(!casual_row.rated);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_active_game_returns_the_ongoing_game_for_either_side(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await;

        let white_view = store.find_active_game(white_id).await.unwrap().unwrap();
        assert_eq!(white_view.id, game_id);
        assert_eq!(white_view.my_side, Side::White);

        let black_view = store.find_active_game(black_id).await.unwrap().unwrap();
        assert_eq!(black_view.id, game_id);
        assert_eq!(black_view.my_side, Side::Black);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_active_game_ignores_finished_and_aborted_games(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;

        let finished_game_id = Uuid::new_v4();
        insert_game_row(&pool, finished_game_id, white_id, black_id, true).await;
        let plan = make_plan(
            finished_game_id,
            white_id,
            black_id,
            true,
            KnownOutcome::Decisive { winner: shakmaty::Color::White },
            GameOverReason::Checkmate,
        );
        store.finalize_game(plan).await.unwrap();

        assert!(store.find_active_game(white_id).await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_active_game_none_when_no_game_in_progress(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let user_id = insert_user(&pool).await;

        assert!(store.find_active_game(user_id).await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_active_game_ids_returns_only_active(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let active_id = Uuid::new_v4();
        insert_game_row(&pool, active_id, white_id, black_id, true).await;

        let finished_id = Uuid::new_v4();
        insert_game_row(&pool, finished_id, white_id, black_id, true).await;
        let plan = make_plan(
            finished_id,
            white_id,
            black_id,
            true,
            KnownOutcome::Decisive { winner: shakmaty::Color::White },
            GameOverReason::Checkmate,
        );
        store.finalize_game(plan).await.unwrap();

        let ids = store.list_active_game_ids().await.unwrap();
        assert_eq!(ids, vec![active_id]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn abort_stale_games_only_touches_given_ids_still_active(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let stale_id = Uuid::new_v4();
        insert_game_row(&pool, stale_id, white_id, black_id, true).await;
        let untouched_id = Uuid::new_v4();
        insert_game_row(&pool, untouched_id, white_id, black_id, true).await;

        let count = store.abort_stale_games(&[stale_id]).await.unwrap();
        assert_eq!(count, 1);

        let stale_status = sqlx::query_scalar!("SELECT status FROM games WHERE id = $1", stale_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stale_status, "aborted");

        let untouched_status =
            sqlx::query_scalar!("SELECT status FROM games WHERE id = $1", untouched_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(untouched_status, "active", "only the given id should be touched");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn abort_stale_games_empty_input_is_noop(pool: PgPool) {
        let store = GameStore::new(pool);
        assert_eq!(store.abort_stale_games(&[]).await.unwrap(), 0);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn persist_progress_writes_moves_and_clocks_for_active_game(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await;

        let moves = vec!["e2e4".to_string(), "e7e5".to_string()];
        let clocks = vec![(299_000, 300_000), (299_000, 298_500)];
        store.persist_progress(game_id, &moves, &clocks).await.unwrap();

        let row = sqlx::query!("SELECT moves, clocks, status FROM games WHERE id = $1", game_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.status, "active");
        assert_eq!(row.moves.as_deref(), Some("e2e4 e7e5"));
        assert_eq!(row.clocks.as_deref(), Some("299000,300000 299000,298500"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn persist_progress_is_noop_once_game_is_finalized(pool: PgPool) {
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

        // A progress write racing behind finalize must not clobber the
        // authoritative finished row.
        let stale_moves = vec!["a2a3".to_string()];
        let stale_clocks = vec![(1, 1)];
        store.persist_progress(game_id, &stale_moves, &stale_clocks).await.unwrap();

        let row = sqlx::query!("SELECT moves, clocks, status FROM games WHERE id = $1", game_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.status, "finished");
        assert_eq!(row.moves.as_deref(), Some("e2e4 e7e5"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn active_games_for_empty_input_skips_query(pool: PgPool) {
        let store = GameStore::new(pool);
        assert!(store.active_games_for(&[]).await.unwrap().is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn active_games_for_finds_both_participants(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let bystander_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await;

        let result = store.active_games_for(&[white_id, black_id, bystander_id]).await.unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[&white_id].game_id, game_id);
        assert_eq!(result[&black_id].game_id, game_id);
        assert!(!result.contains_key(&bystander_id));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn active_games_for_excludes_finished_and_aborted(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await; // 'active'
        let plan = make_plan(
            game_id,
            white_id,
            black_id,
            true,
            KnownOutcome::Decisive { winner: shakmaty::Color::White },
            GameOverReason::Checkmate,
        );
        store.finalize_game(plan).await.unwrap();

        assert!(store.active_games_for(&[white_id, black_id]).await.unwrap().is_empty());
    }

    // ── load_active_game (adoption) ──────────────────────────────────────────

    #[sqlx::test(migrations = "../../migrations")]
    async fn load_active_game_returns_active_row(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await;

        let moves = vec!["e2e4".to_string(), "e7e5".to_string()];
        let clocks = vec![(299_000, 300_000), (299_000, 298_500)];
        store.persist_progress(game_id, &moves, &clocks).await.unwrap();

        let row = store.load_active_game(game_id).await.unwrap().expect("row exists and is active");
        assert_eq!(row.white_user_id, white_id);
        assert_eq!(row.black_user_id, black_id);
        assert!(row.rated);
        assert_eq!(row.moves, moves);
        assert_eq!(row.clocks, clocks);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn load_active_game_none_for_finished(pool: PgPool) {
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

        assert!(store.load_active_game(game_id).await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn load_active_game_none_for_bogus_id(pool: PgPool) {
        let store = GameStore::new(pool);
        assert!(store.load_active_game(Uuid::new_v4()).await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn load_active_game_errs_on_clock_count_mismatch(pool: PgPool) {
        let store = GameStore::new(pool.clone());
        let white_id = insert_user(&pool).await;
        let black_id = insert_user(&pool).await;
        let game_id = Uuid::new_v4();
        insert_game_row(&pool, game_id, white_id, black_id, true).await;

        // Two moves but only one clock pair — malformed, must not silently
        // adopt with wrong/misaligned clocks.
        sqlx::query!(
            "UPDATE games SET moves = 'e2e4 e7e5', clocks = '299000,300000' WHERE id = $1",
            game_id
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(store.load_active_game(game_id).await.is_err());
    }

    #[test]
    fn clocks_from_string_strict_rejects_malformed_pair() {
        assert!(clocks_from_string_strict(Some("not-a-pair"), 1).is_err());
    }

    #[test]
    fn clocks_from_string_strict_accepts_matching_count() {
        let parsed = clocks_from_string_strict(Some("1000,2000 900,1900"), 2).unwrap();
        assert_eq!(parsed, vec![(1000, 2000), (900, 1900)]);
    }
}
