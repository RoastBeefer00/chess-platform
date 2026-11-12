use shared::{
    Entrant, NewTournament, PairingRecord, PairingResult, TournamentFormat, TournamentStatus,
    TournamentSummary, TournamentVenue, TOURNAMENT_MAX_PLAYERS,
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::AuthError;

#[derive(Clone, Debug)]
pub struct TournamentStore {
    pool: PgPool,
}

/// A `tournaments` row plus the two things every reader wants alongside it:
/// the organizer's name and how many people are in.
#[derive(Debug, Clone)]
pub struct TournamentRow {
    pub id: Uuid,
    pub name: String,
    pub format: String,
    pub venue: String,
    pub status: String,
    pub organizer_id: Option<Uuid>,
    pub organizer_username: Option<String>,
    pub schedule_key: Option<String>,
    pub time_initial_seconds: i32,
    pub time_increment_seconds: i32,
    pub rated: bool,
    pub starts_at: OffsetDateTime,
    pub duration_minutes: Option<i32>,
    pub total_rounds: Option<i32>,
    pub current_round: i32,
    pub player_count: i64,
    pub mine: bool,
}

impl TournamentRow {
    // The three `CHECK` constraints on the table mean these can't actually
    // miss; the fallbacks exist so a bad row degrades instead of panicking.
    pub fn format(&self) -> TournamentFormat {
        TournamentFormat::parse(&self.format).unwrap_or(TournamentFormat::Swiss)
    }

    pub fn venue(&self) -> TournamentVenue {
        TournamentVenue::parse(&self.venue).unwrap_or(TournamentVenue::Online)
    }

    pub fn status(&self) -> TournamentStatus {
        TournamentStatus::parse(&self.status).unwrap_or(TournamentStatus::Finished)
    }

    pub fn summary(&self) -> TournamentSummary {
        TournamentSummary {
            id: self.id,
            name: self.name.clone(),
            format: self.format(),
            venue: self.venue(),
            status: self.status(),
            initial_seconds: self.time_initial_seconds,
            increment_seconds: self.time_increment_seconds,
            rated: self.rated,
            starts_at: self.starts_at.unix_timestamp(),
            duration_minutes: self.duration_minutes,
            total_rounds: self.total_rounds,
            current_round: self.current_round,
            player_count: self.player_count.max(0) as u32,
            organizer: self.organizer_username.clone(),
            official: self.schedule_key.is_some(),
            mine: self.mine,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlayerRow {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub seed: i32,
    pub withdrawn: bool,
}

impl PlayerRow {
    pub fn name(&self) -> String {
        self.username
            .clone()
            .or_else(|| self.display_name.clone())
            .unwrap_or_else(|| "Anonymous".to_string())
    }

    pub fn entrant(&self) -> Entrant {
        Entrant { id: self.id, seed: self.seed, withdrawn: self.withdrawn }
    }
}

#[derive(Debug, Clone)]
pub struct PairingRow {
    pub id: Uuid,
    pub round: i32,
    pub white_player_id: Uuid,
    pub black_player_id: Option<Uuid>,
    pub game_id: Option<Uuid>,
    pub result: Option<String>,
}

impl PairingRow {
    pub fn result(&self) -> Option<PairingResult> {
        self.result.as_deref().and_then(PairingResult::parse)
    }

    pub fn record(&self) -> PairingRecord {
        PairingRecord {
            round: self.round,
            white: self.white_player_id,
            black: self.black_player_id,
            result: self.result(),
        }
    }
}

/// One slot of a recurring schedule, for `ensure_scheduled`.
#[derive(Debug, Clone)]
pub struct ScheduledSlot {
    pub key: &'static str,
    pub name: &'static str,
    pub initial_seconds: i32,
    pub increment_seconds: i32,
    pub starts_at: OffsetDateTime,
    pub duration_minutes: i32,
}

pub enum JoinOutcome {
    Joined(Uuid),
    Full,
}

impl TournamentStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Moves arenas across their two clock-driven boundaries: open when the
    /// start time passes, close when the window runs out.
    ///
    /// Nothing runs on a timer to do this. A background sweep would have to
    /// query around the clock, which is exactly what keeps a serverless
    /// Postgres from ever suspending (see the reconcile loop in `main.rs`),
    /// and nobody can observe a stale status without issuing a read anyway —
    /// so the readers call this first and the row is correct by the time
    /// they look at it.
    #[tracing::instrument(skip(self))]
    pub async fn sync_arena_status(&self) -> Result<(), AuthError> {
        sqlx::query!(
            r#"UPDATE tournaments
               SET status = CASE
                       WHEN now() >= starts_at + make_interval(mins => duration_minutes)
                           THEN 'finished'
                       ELSE 'live'
                   END,
                   ended_at = CASE
                       WHEN now() >= starts_at + make_interval(mins => duration_minutes)
                           THEN starts_at + make_interval(mins => duration_minutes)
                       ELSE NULL
                   END
               WHERE format = 'arena'
                 AND status IN ('upcoming', 'live')
                 AND now() >= starts_at
                 AND (status = 'upcoming'
                      OR now() >= starts_at + make_interval(mins => duration_minutes))"#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Creates any of `slots` that don't exist yet. Idempotent across
    /// instances and across calls: `(schedule_key, starts_at)` is unique, so
    /// whoever gets there second inserts nothing.
    #[tracing::instrument(skip(self, slots), fields(n = slots.len()))]
    pub async fn ensure_scheduled(&self, slots: &[ScheduledSlot]) -> Result<(), AuthError> {
        for slot in slots {
            sqlx::query!(
                r#"INSERT INTO tournaments (
                       name, format, venue, schedule_key,
                       time_initial_seconds, time_increment_seconds, rated,
                       starts_at, duration_minutes
                   ) VALUES ($1, 'arena', 'online', $2, $3, $4, TRUE, $5, $6)
                   ON CONFLICT (schedule_key, starts_at) WHERE schedule_key IS NOT NULL
                   DO NOTHING"#,
                slot.name,
                slot.key,
                slot.initial_seconds,
                slot.increment_seconds,
                slot.starts_at,
                slot.duration_minutes,
            )
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// One tournament (`id = Some`) or the browsable list (`id = None`).
    ///
    /// The list is: everything open, plus what finished in the last week.
    /// Over-the-board events only appear to the people involved — a club's
    /// Tuesday night is noise on everybody else's list, and its organizer
    /// shares the link with whoever needs it. Open events older than a month
    /// are treated as abandoned.
    async fn fetch(&self, id: Option<Uuid>, viewer: Option<Uuid>) -> Result<Vec<TournamentRow>, AuthError> {
        let rows = sqlx::query_as!(
            TournamentRow,
            r#"SELECT t.id, t.name, t.format, t.venue, t.status,
                      t.organizer_id, u.username AS "organizer_username?", t.schedule_key,
                      t.time_initial_seconds, t.time_increment_seconds, t.rated,
                      t.starts_at, t.duration_minutes, t.total_rounds, t.current_round,
                      (SELECT count(*) FROM tournament_players p
                        WHERE p.tournament_id = t.id AND NOT p.withdrawn) AS "player_count!",
                      COALESCE(($2::uuid IS NOT NULL AND (
                          t.organizer_id = $2
                          OR EXISTS (SELECT 1 FROM tournament_players p
                                      WHERE p.tournament_id = t.id AND p.user_id = $2)
                      )), false) AS "mine!"
               FROM tournaments t
               LEFT JOIN users u ON u.id = t.organizer_id
               WHERE CASE
                   WHEN $1::uuid IS NOT NULL THEN t.id = $1
                   ELSE (
                       (t.status IN ('upcoming', 'live') AND t.created_at > now() - interval '30 days')
                       OR (t.status = 'finished' AND t.ended_at > now() - interval '7 days')
                   ) AND (
                       t.venue = 'online'
                       OR t.organizer_id = $2
                       OR EXISTS (SELECT 1 FROM tournament_players p
                                   WHERE p.tournament_id = t.id AND p.user_id = $2)
                   )
               END
               ORDER BY (t.schedule_key IS NOT NULL) DESC, (t.status = 'live') DESC, (t.status = 'upcoming') DESC,
                        CASE WHEN t.status = 'finished' THEN t.ended_at END DESC,
                        t.starts_at ASC
               LIMIT 100"#,
            id,
            viewer,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    #[tracing::instrument(skip(self))]
    pub async fn list(&self, viewer: Option<Uuid>) -> Result<Vec<TournamentRow>, AuthError> {
        self.fetch(None, viewer).await
    }

    #[tracing::instrument(skip(self))]
    pub async fn get(&self, id: Uuid, viewer: Option<Uuid>) -> Result<Option<TournamentRow>, AuthError> {
        Ok(self.fetch(Some(id), viewer).await?.into_iter().next())
    }

    /// How many tournaments `organizer` has that aren't over. Caps how many
    /// one account can leave lying around.
    pub async fn count_open_by_organizer(&self, organizer: Uuid) -> Result<i64, AuthError> {
        let n = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM tournaments
               WHERE organizer_id = $1 AND status IN ('upcoming', 'live')"#,
            organizer,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(n)
    }

    #[tracing::instrument(skip(self, new), fields(%organizer))]
    pub async fn create(
        &self,
        organizer: Uuid,
        new: &NewTournament,
        starts_at: OffsetDateTime,
    ) -> Result<Uuid, AuthError> {
        let (duration, rounds) = match new.format {
            TournamentFormat::Arena => (Some(new.duration_minutes as i32), None),
            TournamentFormat::Swiss => (None, Some(new.rounds as i32)),
            TournamentFormat::RoundRobin => (None, None),
        };
        let id = sqlx::query_scalar!(
            r#"INSERT INTO tournaments (
                   name, format, venue, organizer_id,
                   time_initial_seconds, time_increment_seconds, rated,
                   starts_at, duration_minutes, total_rounds
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id"#,
            new.name.trim(),
            new.format.as_str(),
            new.venue.as_str(),
            organizer,
            new.initial_seconds as i32,
            new.increment_seconds as i32,
            new.rated,
            starts_at,
            duration,
            rounds,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// The whole field in seed order, withdrawn players included.
    #[tracing::instrument(skip(self))]
    pub async fn players(&self, tournament_id: Uuid) -> Result<Vec<PlayerRow>, AuthError> {
        let rows = sqlx::query_as!(
            PlayerRow,
            r#"SELECT p.id, p.user_id, u.username AS "username?", p.display_name, p.seed, p.withdrawn
               FROM tournament_players p
               LEFT JOIN users u ON u.id = p.user_id
               WHERE p.tournament_id = $1
               ORDER BY p.seed"#,
            tournament_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Every pairing, oldest first — `shared::tally` reads this as a
    /// timeline.
    #[tracing::instrument(skip(self))]
    pub async fn pairings(&self, tournament_id: Uuid) -> Result<Vec<PairingRow>, AuthError> {
        let rows = sqlx::query_as!(
            PairingRow,
            r#"SELECT id, round, white_player_id, black_player_id, game_id, result
               FROM tournament_pairings
               WHERE tournament_id = $1
               ORDER BY created_at, id"#,
            tournament_id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Shared by both ways into a tournament. The seed is "one past the
    /// highest", read and written in one statement; two people joining at
    /// the same instant can still compute the same seed, which the
    /// `(tournament_id, seed)` unique index turns into a retry rather than a
    /// duplicate.
    async fn insert_player(
        &self,
        tournament_id: Uuid,
        user_id: Option<Uuid>,
        display_name: Option<&str>,
    ) -> Result<JoinOutcome, AuthError> {
        let count = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM tournament_players WHERE tournament_id = $1"#,
            tournament_id,
        )
        .fetch_one(&self.pool)
        .await?;
        if count as usize >= TOURNAMENT_MAX_PLAYERS {
            return Ok(JoinOutcome::Full);
        }

        let mut attempts = 0;
        loop {
            let result = sqlx::query_scalar!(
                r#"INSERT INTO tournament_players (tournament_id, user_id, display_name, seed)
                   SELECT $1, $2, $3, COALESCE(MAX(seed), -1) + 1
                   FROM tournament_players WHERE tournament_id = $1
                   ON CONFLICT (tournament_id, user_id) DO UPDATE SET withdrawn = FALSE
                   RETURNING id"#,
                tournament_id,
                user_id,
                display_name,
            )
            .fetch_one(&self.pool)
            .await;
            match result {
                Ok(id) => return Ok(JoinOutcome::Joined(id)),
                Err(sqlx::Error::Database(e)) if e.is_unique_violation() && attempts < 5 => {
                    attempts += 1;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Enters `user_id`, or re-enters them if they had withdrawn.
    #[tracing::instrument(skip(self))]
    pub async fn join(&self, tournament_id: Uuid, user_id: Uuid) -> Result<JoinOutcome, AuthError> {
        self.insert_player(tournament_id, Some(user_id), None).await
    }

    /// Enters someone with no account — an over-the-board walk-in.
    #[tracing::instrument(skip(self, name))]
    pub async fn add_named_player(&self, tournament_id: Uuid, name: &str) -> Result<JoinOutcome, AuthError> {
        self.insert_player(tournament_id, None, Some(name)).await
    }

    /// Flags (or un-flags) a player as out. They keep their results and
    /// their row; they just stop being paired.
    pub async fn set_withdrawn(
        &self,
        tournament_id: Uuid,
        player_id: Uuid,
        withdrawn: bool,
    ) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            "UPDATE tournament_players SET withdrawn = $3 WHERE tournament_id = $1 AND id = $2",
            tournament_id,
            player_id,
            withdrawn,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Deletes a player outright. Only meaningful before the first round —
    /// the caller checks; afterwards it would take their games with it.
    pub async fn remove_player(&self, tournament_id: Uuid, player_id: Uuid) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            "DELETE FROM tournament_players WHERE tournament_id = $1 AND id = $2",
            tournament_id,
            player_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// `upcoming` → `live`. `false` if it wasn't upcoming — somebody else
    /// already started it, which makes a double-clicked Start harmless.
    #[tracing::instrument(skip(self))]
    pub async fn start(&self, tournament_id: Uuid, total_rounds: i32) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            r#"UPDATE tournaments
               SET status = 'live', total_rounds = $2, starts_at = now()
               WHERE id = $1 AND status = 'upcoming'"#,
            tournament_id,
            total_rounds,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Adds one round to a running tournament that has just finished its
    /// last, for a tie-break. Succeeds only while the tournament is still on
    /// `final_round` — the round the caller saw — so a double click, or a stale
    /// tab, adds one round, not two (the new round is itself "the final round"
    /// until it is played, which a bare "is on its last round" check would
    /// mistake for a fresh chance).
    #[tracing::instrument(skip(self))]
    pub async fn add_round(&self, tournament_id: Uuid, final_round: i32) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            r#"UPDATE tournaments SET total_rounds = $2 + 1
               WHERE id = $1 AND status = 'live'
                 AND current_round = $2 AND total_rounds = $2"#,
            tournament_id,
            final_round,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// How many tournaments `organizer` has created since `since`.
    #[tracing::instrument(skip(self))]
    pub async fn count_created_since(&self, organizer: Uuid, since: OffsetDateTime) -> Result<i64, AuthError> {
        Ok(sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM tournaments WHERE organizer_id = $1 AND created_at > $2"#,
            organizer,
            since,
        )
        .fetch_one(&self.pool)
        .await?)
    }

    /// Opens round `round` and records its pairings, atomically.
    ///
    /// The round counter only advances from exactly `round - 1`, so two
    /// organizers' tabs (or one impatient double click) can't both create
    /// the same round: the second finds the counter already moved and gets
    /// `None`. Byes are settled on the spot; real games start undecided.
    ///
    /// Returns the new pairings' ids, index-aligned with `pairings`.
    #[tracing::instrument(skip(self, pairings), fields(n = pairings.len()))]
    pub async fn open_round(
        &self,
        tournament_id: Uuid,
        round: i32,
        pairings: &[(Uuid, Option<Uuid>)],
    ) -> Result<Option<Vec<Uuid>>, AuthError> {
        let mut tx = self.pool.begin().await?;
        let advanced = sqlx::query!(
            r#"UPDATE tournaments SET current_round = $2
               WHERE id = $1 AND status = 'live' AND current_round = $2 - 1"#,
            tournament_id,
            round,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if advanced == 0 {
            return Ok(None);
        }

        let mut ids = Vec::with_capacity(pairings.len());
        for (white, black) in pairings {
            let result = black.is_none().then_some("bye");
            let id = sqlx::query_scalar!(
                r#"INSERT INTO tournament_pairings
                       (tournament_id, round, white_player_id, black_player_id, result)
                   VALUES ($1, $2, $3, $4, $5)
                   RETURNING id"#,
                tournament_id,
                round,
                white,
                *black,
                result,
            )
            .fetch_one(&mut *tx)
            .await?;
            ids.push(id);
        }
        tx.commit().await?;
        Ok(Some(ids))
    }

    /// Links a pairing to the game that will decide it.
    pub async fn attach_game(&self, pairing_id: Uuid, game_id: Uuid) -> Result<(), AuthError> {
        sqlx::query!(
            "UPDATE tournament_pairings SET game_id = $2 WHERE id = $1",
            pairing_id,
            game_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Records one arena game. Arenas have no rounds, so everything is
    /// round 0.
    pub async fn insert_arena_pairing(
        &self,
        tournament_id: Uuid,
        white: Uuid,
        black: Uuid,
        game_id: Uuid,
    ) -> Result<(), AuthError> {
        sqlx::query!(
            r#"INSERT INTO tournament_pairings
                   (tournament_id, round, white_player_id, black_player_id, game_id)
               VALUES ($1, 0, $2, $3, $4)"#,
            tournament_id,
            white,
            black,
            game_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// An organizer entering or correcting a result.
    ///
    /// Never touches a bye, and never a game that is still being played on
    /// the site: that one will report its own result, and letting a click
    /// race it would make the score depend on who was faster. Once the game
    /// has reported (including an abort, which lands as `void`) the
    /// organizer's word is final — that is how a forfeit gets awarded.
    #[tracing::instrument(skip(self))]
    pub async fn set_result(
        &self,
        tournament_id: Uuid,
        pairing_id: Uuid,
        result: Option<PairingResult>,
    ) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            r#"UPDATE tournament_pairings
               SET result = $3
               WHERE tournament_id = $1 AND id = $2
                 AND black_player_id IS NOT NULL
                 AND (game_id IS NULL OR result IS NOT NULL)"#,
            tournament_id,
            pairing_id,
            result.map(|r| r.as_str()),
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    #[tracing::instrument(skip(self))]
    pub async fn finish(&self, tournament_id: Uuid) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            r#"UPDATE tournaments SET status = 'finished', ended_at = now()
               WHERE id = $1 AND status = 'live'"#,
            tournament_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Calls off a tournament that hasn't started.
    #[tracing::instrument(skip(self))]
    pub async fn cancel(&self, tournament_id: Uuid) -> Result<bool, AuthError> {
        let result = sqlx::query!(
            r#"UPDATE tournaments SET status = 'cancelled', ended_at = now()
               WHERE id = $1 AND status = 'upcoming'"#,
            tournament_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Which tournament `game_id` belongs to, if any.
    pub async fn tournament_for_game(&self, game_id: Uuid) -> Result<Option<Uuid>, AuthError> {
        let id = sqlx::query_scalar!(
            "SELECT tournament_id FROM tournament_pairings WHERE game_id = $1",
            game_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(id)
    }

    /// `user_id`'s in-progress game in this tournament, if they have one.
    pub async fn active_game_of(&self, tournament_id: Uuid, user_id: Uuid) -> Result<Option<Uuid>, AuthError> {
        let id = sqlx::query_scalar!(
            r#"SELECT g.id
               FROM tournament_pairings p
               JOIN games g ON g.id = p.game_id
               WHERE p.tournament_id = $1
                 AND g.status = 'active'
                 AND (g.white_user_id = $2 OR g.black_user_id = $2)
               LIMIT 1"#,
            tournament_id,
            user_id,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(id)
    }

    /// Takes the per-tournament pairing lock, or returns `None` if another
    /// instance holds it.
    ///
    /// An arena is paired by whichever instances happen to have people on
    /// its page, so several can be running the same loop. A Postgres
    /// advisory lock scoped to this transaction makes exactly one of them
    /// pair at a time, and it cannot leak: the lock goes when the
    /// transaction does, however that happens — commit, drop, or the
    /// process dying mid-pass.
    pub async fn try_pairing_lock(
        &self,
        tournament_id: Uuid,
    ) -> Result<Option<sqlx::Transaction<'static, sqlx::Postgres>>, AuthError> {
        // Advisory locks are keyed by a bigint; the first half of a v4 UUID
        // is as good a 64-bit key as any.
        let key = i64::from_be_bytes(
            tournament_id.as_bytes()[..8]
                .try_into()
                .expect("a UUID is 16 bytes"),
        );
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query_scalar!(r#"SELECT pg_try_advisory_xact_lock($1) AS "locked!""#, key)
            .fetch_one(&mut *tx)
            .await?;
        Ok(locked.then_some(tx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::tally;

    async fn insert_user(pool: &PgPool, username: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO users (id, email, username) VALUES ($1, $2, $3)",
            id,
            format!("{id}@test.invalid"),
            username,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    fn swiss(venue: TournamentVenue) -> NewTournament {
        NewTournament {
            name: "Club night".into(),
            format: TournamentFormat::Swiss,
            venue,
            initial_seconds: 300,
            increment_seconds: 3,
            rated: false,
            starts_in_minutes: 0,
            duration_minutes: 60,
            rounds: 3,
        }
    }

    async fn joined(store: &TournamentStore, t: Uuid, user: Uuid) -> Uuid {
        match store.join(t, user).await.unwrap() {
            JoinOutcome::Joined(id) => id,
            JoinOutcome::Full => panic!("tournament unexpectedly full"),
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn join_is_idempotent_and_rejoining_clears_a_withdrawal(pool: PgPool) {
        let store = TournamentStore::new(pool.clone());
        let org = insert_user(&pool, "organizer").await;
        let alice = insert_user(&pool, "alice").await;
        let t = store.create(org, &swiss(TournamentVenue::Online), OffsetDateTime::now_utc()).await.unwrap();

        let first = joined(&store, t, alice).await;
        let second = joined(&store, t, alice).await;
        assert_eq!(first, second, "joining twice must not create a second entry");

        assert!(store.set_withdrawn(t, first, true).await.unwrap());
        assert_eq!(store.get(t, None).await.unwrap().unwrap().player_count, 0);
        joined(&store, t, alice).await;
        let players = store.players(t).await.unwrap();
        assert_eq!(players.len(), 1);
        assert!(!players[0].withdrawn);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn seeds_follow_join_order_across_accounts_and_walk_ins(pool: PgPool) {
        let store = TournamentStore::new(pool.clone());
        let org = insert_user(&pool, "organizer").await;
        let alice = insert_user(&pool, "alice").await;
        let t = store.create(org, &swiss(TournamentVenue::Otb), OffsetDateTime::now_utc()).await.unwrap();

        joined(&store, t, alice).await;
        store.add_named_player(t, "Bob from the club").await.unwrap();
        store.add_named_player(t, "Carol").await.unwrap();

        let players = store.players(t).await.unwrap();
        let names: Vec<String> = players.iter().map(PlayerRow::name).collect();
        assert_eq!(names, vec!["alice", "Bob from the club", "Carol"]);
        assert_eq!(players.iter().map(|p| p.seed).collect::<Vec<_>>(), vec![0, 1, 2]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_round_can_only_be_opened_once(pool: PgPool) {
        let store = TournamentStore::new(pool.clone());
        let org = insert_user(&pool, "organizer").await;
        let t = store.create(org, &swiss(TournamentVenue::Otb), OffsetDateTime::now_utc()).await.unwrap();
        let JoinOutcome::Joined(a) = store.add_named_player(t, "A").await.unwrap() else { panic!() };
        let JoinOutcome::Joined(b) = store.add_named_player(t, "B").await.unwrap() else { panic!() };
        let JoinOutcome::Joined(c) = store.add_named_player(t, "C").await.unwrap() else { panic!() };

        // Not live yet: no round can open.
        assert!(store.open_round(t, 1, &[(a, Some(b)), (c, None)]).await.unwrap().is_none());

        assert!(store.start(t, 3).await.unwrap());
        assert!(!store.start(t, 3).await.unwrap(), "a second start is a no-op");

        let ids = store.open_round(t, 1, &[(a, Some(b)), (c, None)]).await.unwrap().unwrap();
        assert_eq!(ids.len(), 2);
        // The double click.
        assert!(store.open_round(t, 1, &[(a, Some(b)), (c, None)]).await.unwrap().is_none());
        // And skipping ahead.
        assert!(store.open_round(t, 3, &[(a, Some(c))]).await.unwrap().is_none());

        let pairings = store.pairings(t).await.unwrap();
        assert_eq!(pairings.len(), 2);
        let bye = pairings.iter().find(|p| p.black_player_id.is_none()).unwrap();
        assert_eq!(bye.result(), Some(PairingResult::Bye), "a bye is settled as it is created");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn organizer_results_feed_standings_but_cannot_touch_byes(pool: PgPool) {
        let store = TournamentStore::new(pool.clone());
        let org = insert_user(&pool, "organizer").await;
        let t = store.create(org, &swiss(TournamentVenue::Otb), OffsetDateTime::now_utc()).await.unwrap();
        let JoinOutcome::Joined(a) = store.add_named_player(t, "A").await.unwrap() else { panic!() };
        let JoinOutcome::Joined(b) = store.add_named_player(t, "B").await.unwrap() else { panic!() };
        let JoinOutcome::Joined(c) = store.add_named_player(t, "C").await.unwrap() else { panic!() };
        store.start(t, 3).await.unwrap();
        let ids = store.open_round(t, 1, &[(a, Some(b)), (c, None)]).await.unwrap().unwrap();

        assert!(store.set_result(t, ids[0], Some(PairingResult::Black)).await.unwrap());
        assert!(!store.set_result(t, ids[1], Some(PairingResult::Draw)).await.unwrap());

        let players: Vec<_> = store.players(t).await.unwrap().iter().map(PlayerRow::entrant).collect();
        let pairings: Vec<_> = store.pairings(t).await.unwrap().iter().map(PairingRow::record).collect();
        let ranked = tally(&players, &pairings);
        let score = |id: Uuid| ranked.iter().find(|x| x.id == id).unwrap().score2;
        assert_eq!((score(a), score(b), score(c)), (0, 2, 2));

        // A correction replaces the result rather than adding to it.
        assert!(store.set_result(t, ids[0], Some(PairingResult::Draw)).await.unwrap());
        let pairings: Vec<_> = store.pairings(t).await.unwrap().iter().map(PairingRow::record).collect();
        let ranked = tally(&players, &pairings);
        let score = |id: Uuid| ranked.iter().find(|x| x.id == id).unwrap().score2;
        assert_eq!((score(a), score(b)), (1, 1));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn scheduled_slots_are_created_once(pool: PgPool) {
        let store = TournamentStore::new(pool.clone());
        let slot = ScheduledSlot {
            key: "daily-blitz",
            name: "Daily Blitz Arena",
            initial_seconds: 180,
            increment_seconds: 2,
            starts_at: OffsetDateTime::now_utc() + time::Duration::hours(1),
            duration_minutes: 45,
        };
        let slots = [slot];
        store.ensure_scheduled(&slots).await.unwrap();
        store.ensure_scheduled(&slots).await.unwrap();

        let list = store.list(None).await.unwrap();
        assert_eq!(list.len(), 1);
        let s = list[0].summary();
        assert!(s.official && s.rated);
        assert_eq!(s.status, TournamentStatus::Upcoming);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn arenas_open_and_close_on_the_clock(pool: PgPool) {
        let store = TournamentStore::new(pool.clone());
        let now = OffsetDateTime::now_utc();
        let slot = |key: &'static str, starts_at: OffsetDateTime| ScheduledSlot {
            key,
            name: "Arena",
            initial_seconds: 180,
            increment_seconds: 0,
            starts_at,
            duration_minutes: 30,
        };
        store
            .ensure_scheduled(&[
                slot("future", now + time::Duration::hours(1)),
                slot("running", now - time::Duration::minutes(10)),
                slot("over", now - time::Duration::hours(2)),
            ])
            .await
            .unwrap();
        store.sync_arena_status().await.unwrap();

        let status_of = |key: &'static str, rows: &[TournamentRow]| {
            rows.iter().find(|r| r.schedule_key.as_deref() == Some(key)).map(|r| r.status())
        };
        let rows = store.list(None).await.unwrap();
        assert_eq!(status_of("future", &rows), Some(TournamentStatus::Upcoming));
        assert_eq!(status_of("running", &rows), Some(TournamentStatus::Live));
        assert_eq!(status_of("over", &rows), Some(TournamentStatus::Finished));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn otb_events_are_listed_only_for_the_people_in_them(pool: PgPool) {
        let store = TournamentStore::new(pool.clone());
        let org = insert_user(&pool, "organizer").await;
        let player = insert_user(&pool, "player").await;
        let stranger = insert_user(&pool, "stranger").await;
        let t = store.create(org, &swiss(TournamentVenue::Otb), OffsetDateTime::now_utc()).await.unwrap();
        joined(&store, t, player).await;

        assert!(store.list(None).await.unwrap().is_empty());
        assert!(store.list(Some(stranger)).await.unwrap().is_empty());
        assert_eq!(store.list(Some(org)).await.unwrap().len(), 1);
        let for_player = store.list(Some(player)).await.unwrap();
        assert_eq!(for_player.len(), 1);
        assert!(for_player[0].mine);
        // The link still works for anyone who has it.
        assert!(store.get(t, Some(stranger)).await.unwrap().is_some());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_pairing_lock_excludes_a_second_holder(pool: PgPool) {
        let store = TournamentStore::new(pool.clone());
        let t = Uuid::new_v4();
        let held = store.try_pairing_lock(t).await.unwrap().expect("first taker gets the lock");
        assert!(store.try_pairing_lock(t).await.unwrap().is_none());
        // A different tournament is unaffected.
        assert!(store.try_pairing_lock(Uuid::new_v4()).await.unwrap().is_some());
        held.commit().await.unwrap();
        assert!(store.try_pairing_lock(t).await.unwrap().is_some());
    }

    /// A scheduled arena has no organizer, so `organizer_id = viewer` is NULL
    /// rather than false — which once made the whole list fail to decode for
    /// anyone signed in.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_signed_in_viewer_can_list_organizerless_arenas(pool: PgPool) {
        let store = TournamentStore::new(pool.clone());
        let viewer = insert_user(&pool, "viewer").await;
        store
            .ensure_scheduled(&[ScheduledSlot {
                key: "test-arena",
                name: "Test Arena",
                initial_seconds: 180,
                increment_seconds: 0,
                starts_at: OffsetDateTime::now_utc() + time::Duration::hours(1),
                duration_minutes: 30,
            }])
            .await
            .unwrap();
        let listed = store.list(Some(viewer)).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].mine);
        assert!(!store.get(listed[0].id, Some(viewer)).await.unwrap().unwrap().mine);
    }
}
