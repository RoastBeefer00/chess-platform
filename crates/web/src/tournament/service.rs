//! Running tournaments: the scheduled dailies, round generation, and the
//! arena pairing loop.
//!
//! The rules for *who plays whom* are pure functions in `shared::tournament`;
//! the rows are `db::TournamentStore`. This is the part in between that has
//! to know about game rooms, Redis and the clock.
//!
//! One constraint shapes all of it: **nothing here runs unless somebody is
//! looking.** There is no scheduler thread. Scheduled tournaments are created
//! by the first person to open the list; arenas change status when someone
//! reads them; an arena's pairing loop exists only on instances that have its
//! entrants polling, and exits when they leave. An idle deployment therefore
//! issues no queries and lets a serverless Postgres suspend — the same rule
//! the reconcile loop in `main.rs` follows, for the same bill.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::Duration;

use leptos::prelude::ServerFnError;
use shared::{
    arena_pairings, round_robin_round, round_robin_rounds, swiss_pairings, tally, GameConfig,
    PairingView, RatingMode, StandingRow, TimeControl, TimeMode, TournamentFormat,
    TournamentStatus, TournamentVenue, TournamentView, TournamentViewer, Variant,
    ROUND_ROBIN_MAX_PLAYERS,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::User;
use crate::db::{PairingRow, PlayerRow, ScheduledSlot, TournamentRow};
use crate::state::AppState;

/// The site's own recurring arenas: `(key, name, base seconds, increment
/// seconds, UTC hour, minutes long)`. One of each per day.
///
/// Evening in Europe, afternoon in the Americas — the hours with the most
/// people awake at once. Adding a row here is all it takes to add an event;
/// `ensure_schedule` picks it up within the hour.
const DAILY: &[(&str, &str, i32, i32, u8, i32)] = &[
    ("daily-bullet", "Daily Bullet Arena", 60, 0, 17, 30),
    ("daily-blitz", "Daily Blitz Arena", 180, 2, 19, 45),
    ("daily-rapid", "Daily Rapid Arena", 600, 0, 21, 60),
];

/// How often an arena's pairing loop looks for people to pair.
const ARENA_PAIR_INTERVAL: Duration = Duration::from_secs(3);
/// How many consecutive passes with nobody to pair before the loop gives up
/// and exits (about a minute). The next entrant to poll starts it again.
const ARENA_IDLE_PASSES: u32 = 20;
/// How many of an arena's most recent games the page shows.
const ARENA_RECENT_GAMES: usize = 30;

fn err(msg: &'static str) -> ServerFnError {
    ServerFnError::new(msg)
}

/// Makes sure today's and tomorrow's scheduled arenas exist.
///
/// Called by the list page. Memoised per instance per UTC hour, so in
/// practice it is six idempotent inserts an hour on an instance someone is
/// using and nothing at all on one nobody is.
pub async fn ensure_schedule(state: &AppState) {
    let now = OffsetDateTime::now_utc();
    let hour = now.unix_timestamp() / 3600;
    if state.schedule_checked_hour.swap(hour, Ordering::Relaxed) == hour {
        return;
    }

    let midnight = now.replace_time(time::Time::MIDNIGHT);
    let mut slots = Vec::new();
    for day in 0..2 {
        for &(key, name, initial_seconds, increment_seconds, hour_utc, duration_minutes) in DAILY {
            let starts_at = midnight + time::Duration::days(day) + time::Duration::hours(i64::from(hour_utc));
            // Don't conjure up an event that is already over.
            if starts_at + time::Duration::minutes(i64::from(duration_minutes)) <= now {
                continue;
            }
            slots.push(ScheduledSlot { key, name, initial_seconds, increment_seconds, starts_at, duration_minutes });
        }
    }

    if let Err(e) = state.tournament_store.ensure_scheduled(&slots).await {
        tracing::warn!(?e, "failed to ensure scheduled tournaments");
        // Forget the memo so the next list view retries instead of waiting
        // out the hour.
        state.schedule_checked_hour.store(-1, Ordering::Relaxed);
    }
}

fn game_config(row: &TournamentRow) -> GameConfig {
    GameConfig {
        time_control: TimeControl {
            initial_time: i64::from(row.time_initial_seconds) * 1000,
            mode: TimeMode::Increment(i64::from(row.time_increment_seconds) * 1000),
        },
        variant: Variant::Standard,
        rated: if row.rated { RatingMode::Rated } else { RatingMode::Casual },
    }
}

/// Everything the tournament page shows, for `viewer`.
///
/// This is also the arena's heartbeat. A live arena pairs whoever is *on the
/// page*, and the page polling this is the proof that they are — so for an
/// entrant, reading the tournament is what keeps them in the pairing pool
/// and keeps a pairing loop alive on this instance.
#[tracing::instrument(skip(state, viewer), fields(viewer = ?viewer.map(|u| u.id)))]
pub async fn build_view(
    state: &AppState,
    id: Uuid,
    viewer: Option<&User>,
) -> Result<TournamentView, ServerFnError> {
    let store = &state.tournament_store;
    store.sync_arena_status().await?;
    let row = store
        .get(id, viewer.map(|u| u.id))
        .await?
        .ok_or_else(|| err("tournament not found"))?;
    let players = store.players(id).await?;
    let pairings = store.pairings(id).await?;

    let summary = row.summary();
    let names: HashMap<Uuid, &PlayerRow> = players.iter().map(|p| (p.id, p)).collect();

    let entrants: Vec<_> = players.iter().map(PlayerRow::entrant).collect();
    let records: Vec<_> = pairings.iter().map(PairingRow::record).collect();
    let standings: Vec<StandingRow> = tally(&entrants, &records)
        .into_iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let p = names.get(&t.id)?;
            Some(StandingRow {
                rank: i as u32 + 1,
                player_id: t.id,
                username: p.username.clone(),
                name: p.name(),
                withdrawn: t.withdrawn,
                score2: t.score2,
                played: t.played,
                wins: t.wins,
                draws: t.draws,
                losses: t.losses,
                tiebreak2: t.tiebreak2,
            })
        })
        .collect();

    let to_view = |p: &PairingRow| -> Option<PairingView> {
        Some(PairingView {
            id: p.id,
            round: p.round,
            white_id: p.white_player_id,
            white_name: names.get(&p.white_player_id)?.name(),
            black_id: p.black_player_id,
            black_name: match p.black_player_id {
                Some(b) => Some(names.get(&b)?.name()),
                None => None,
            },
            result: p.result(),
            game_id: p.game_id,
        })
    };
    let pairing_views: Vec<PairingView> = match summary.format {
        // Newest first, and only the tail: a busy arena plays hundreds.
        TournamentFormat::Arena => pairings.iter().rev().take(ARENA_RECENT_GAMES).filter_map(to_view).collect(),
        _ => pairings.iter().filter_map(to_view).collect(),
    };

    let mut view_viewer = TournamentViewer::default();
    if let Some(user) = viewer {
        let me = players.iter().find(|p| p.user_id == Some(user.id));
        view_viewer = TournamentViewer {
            signed_in: true,
            can_enter: !user.is_guest && user.username.is_some(),
            player_id: me.map(|p| p.id),
            withdrawn: me.is_some_and(|p| p.withdrawn),
            is_organizer: row.organizer_id == Some(user.id),
            active_game: None,
        };

        let playing = me.is_some_and(|p| !p.withdrawn);
        if playing && summary.venue == TournamentVenue::Online && summary.status == TournamentStatus::Live {
            view_viewer.active_game = store.active_game_of(id, user.id).await?;
            if summary.format == TournamentFormat::Arena {
                state.redis_client.lobby_touch(id, user.id).await;
                ensure_arena_runner(state, &row);
            }
        }
    }

    Ok(TournamentView {
        summary,
        standings,
        pairings: pairing_views,
        viewer: view_viewer,
        server_now: OffsetDateTime::now_utc().unix_timestamp(),
    })
}

/// Starts a swiss or round robin: fixes its length, flips it live, and pairs
/// round one.
pub async fn start(state: &AppState, row: &TournamentRow) -> Result<(), ServerFnError> {
    if row.status() != TournamentStatus::Upcoming {
        return Err(err("this tournament has already started"));
    }
    let format = row.format();
    if format == TournamentFormat::Arena {
        return Err(err("an arena starts by itself at its start time"));
    }

    let players = state.tournament_store.players(row.id).await?;
    let field = players.iter().filter(|p| !p.withdrawn).count();
    if field < 2 {
        return Err(err("a tournament needs at least two players"));
    }
    let total_rounds = match format {
        TournamentFormat::RoundRobin => {
            if field > ROUND_ROBIN_MAX_PLAYERS {
                return Err(err("a round robin takes at most 16 players — use swiss for a bigger field"));
            }
            round_robin_rounds(players.len()) as i32
        }
        _ => row.total_rounds.unwrap_or(1),
    };

    if !state.tournament_store.start(row.id, total_rounds).await? {
        return Err(err("this tournament has already started"));
    }

    // Re-read rather than patch the stale row: `open_next_round` keys off
    // the status and round counter the database now holds.
    let row = state
        .tournament_store
        .get(row.id, None)
        .await?
        .ok_or_else(|| err("tournament not found"))?;
    open_next_round(state, &row).await
}

/// Pairs the next round of a live swiss or round robin — or, if the last
/// round is complete, finishes the tournament.
///
/// Refuses while any game of the current round is undecided: a swiss pairs
/// on scores, so a round paired early is a round paired wrong.
#[tracing::instrument(skip(state, row), fields(tournament_id = %row.id, round = row.current_round + 1))]
pub async fn open_next_round(state: &AppState, row: &TournamentRow) -> Result<(), ServerFnError> {
    let store = &state.tournament_store;
    if row.status() != TournamentStatus::Live {
        return Err(err("this tournament isn't running"));
    }
    let format = row.format();
    if format == TournamentFormat::Arena {
        return Err(err("an arena has no rounds"));
    }

    let players = store.players(row.id).await?;
    let pairings = store.pairings(row.id).await?;
    if pairings.iter().any(|p| p.round == row.current_round && p.result.is_none()) {
        return Err(err("the current round still has undecided games"));
    }

    let next = row.current_round + 1;
    if row.total_rounds.is_some_and(|total| next > total) {
        store.finish(row.id).await?;
        return Ok(());
    }

    let entrants: Vec<_> = players.iter().map(PlayerRow::entrant).collect();
    let pairs = match format {
        TournamentFormat::RoundRobin => round_robin_round(&entrants, next as u32),
        _ => {
            let records: Vec<_> = pairings.iter().map(PairingRow::record).collect();
            swiss_pairings(&tally(&entrants, &records))
        }
    };
    if !pairs.iter().any(|(_, black)| black.is_some()) {
        return Err(err("not enough active players to pair a round"));
    }

    let by_id: HashMap<Uuid, &PlayerRow> = players.iter().map(|p| (p.id, p)).collect();
    let online = row.venue() == TournamentVenue::Online;

    if online {
        // A player can be in one game at a time (`find_active_game` and the
        // reconnect banner are built on it). Starting the round anyway would
        // hand someone a second board, so make the organizer wait instead.
        let user_ids: Vec<Uuid> = pairs
            .iter()
            .filter(|(_, black)| black.is_some())
            .flat_map(|(white, black)| [Some(*white), *black])
            .flatten()
            .filter_map(|pid| by_id.get(&pid).and_then(|p| p.user_id))
            .collect();
        let busy = state.game_store.active_games_for(&user_ids).await?;
        if !busy.is_empty() {
            return Err(err("some players are still in a game — try again when they finish"));
        }
    }

    let Some(pairing_ids) = store.open_round(row.id, next, &pairs).await? else {
        return Err(err("that round has already been started"));
    };
    tracing::info!(tournament_id = %row.id, round = next, games = pairs.len(), "tournament_round_opened");

    if online {
        let config = game_config(row);
        for (pairing_id, (white, black)) in pairing_ids.into_iter().zip(pairs) {
            let Some(black) = black else { continue };
            let (Some(white_user), Some(black_user)) = (
                by_id.get(&white).and_then(|p| p.user_id),
                by_id.get(&black).and_then(|p| p.user_id),
            ) else {
                continue;
            };
            // A failure here leaves the pairing with no game and no result,
            // which the organizer can settle by hand — better than failing
            // the whole round after half its games already exist.
            match state.create_tournament_game(config.clone(), white_user, black_user, row.id).await {
                Ok(game_id) => {
                    if let Err(e) = store.attach_game(pairing_id, game_id).await {
                        tracing::error!(?e, %pairing_id, %game_id, "failed to attach game to pairing");
                    }
                }
                Err(e) => tracing::error!(?e, %pairing_id, "failed to create tournament game"),
            }
        }
    }
    Ok(())
}

/// An extra round after the last one, pairing only the players tied for a
/// podium place so the tie is settled over the board.
///
/// Over-the-board only: online, the tiebreaks (Buchholz, wins) decide, and
/// there is nobody to hand a board to. An odd group leaves its lowest-ranked
/// member out of the round — a bye would hand them a free point and decide
/// the tie by accident. The organizer can add another round if it is still
/// level afterwards.
pub async fn open_tiebreak_round(state: &AppState, row: &TournamentRow) -> Result<(), ServerFnError> {
    let store = &state.tournament_store;
    if row.status() != TournamentStatus::Live || row.venue() != TournamentVenue::Otb {
        return Err(err("tie-break rounds are for over-the-board tournaments in progress"));
    }
    if row.format() == TournamentFormat::Arena || row.total_rounds != Some(row.current_round) || row.current_round < 1 {
        return Err(err("a tie-break round can only be added after the last round"));
    }

    let players = store.players(row.id).await?;
    let pairings = store.pairings(row.id).await?;
    if pairings.iter().any(|p| p.round == row.current_round && p.result.is_none()) {
        return Err(err("the current round still has undecided games"));
    }

    let entrants: Vec<_> = players.iter().map(PlayerRow::entrant).collect();
    let records: Vec<_> = pairings.iter().map(PairingRow::record).collect();
    let ranked: Vec<_> = tally(&entrants, &records).into_iter().filter(|t| !t.withdrawn).collect();
    let scores: Vec<(Uuid, u32)> = ranked.iter().map(|t| (t.id, t.score2)).collect();
    let mut group = shared::tiebreak_group(&scores);
    if group.len() % 2 == 1 {
        group.pop();
    }
    if group.len() < 2 {
        return Err(err("nobody is tied for a podium place"));
    }
    let in_group: HashSet<Uuid> = group.iter().copied().collect();
    let playoff: Vec<_> = ranked.into_iter().filter(|t| in_group.contains(&t.id)).collect();
    let pairs = swiss_pairings(&playoff);

    if !store.add_round(row.id, row.current_round).await? {
        return Err(err("that round has already been added"));
    }
    let Some(_) = store.open_round(row.id, row.current_round + 1, &pairs).await? else {
        return Err(err("that round has already been started"));
    };
    tracing::info!(tournament_id = %row.id, round = row.current_round + 1, "tournament_tiebreak_round");
    Ok(())
}

enum Pass {
    /// Looked for players and there was something to do (or another
    /// instance was already doing it).
    Busy,
    /// Fewer than two people waiting.
    Quiet,
    /// The arena's window has closed.
    Over,
}

/// Starts this instance's pairing loop for a live arena, if it isn't
/// already running one.
///
/// Cheap enough to call on every poll: the common case is a set lookup that
/// finds the loop already there.
pub fn ensure_arena_runner(state: &AppState, row: &TournamentRow) {
    let id = row.id;
    {
        let mut running = state.arena_runners.lock().unwrap_or_else(|e| e.into_inner());
        if !running.insert(id) {
            return;
        }
    }

    let Some(duration_minutes) = row.duration_minutes else {
        state.arena_runners.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        return;
    };
    let ends_at = row.starts_at + time::Duration::minutes(i64::from(duration_minutes));
    let config = game_config(row);
    let state = state.clone();

    tokio::spawn(async move {
        tracing::info!(tournament_id = %id, "arena pairing loop started");
        let mut quiet_passes = 0;
        loop {
            match arena_pass(&state, id, ends_at, &config).await {
                Ok(Pass::Over) => break,
                Ok(Pass::Busy) => quiet_passes = 0,
                Ok(Pass::Quiet) => quiet_passes += 1,
                Err(e) => {
                    tracing::warn!(?e, tournament_id = %id, "arena pairing pass failed");
                    quiet_passes += 1;
                }
            }
            if quiet_passes >= ARENA_IDLE_PASSES {
                break;
            }
            tokio::time::sleep(ARENA_PAIR_INTERVAL).await;
        }
        state.arena_runners.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        tracing::info!(tournament_id = %id, "arena pairing loop stopped");
    });
}

/// One look at the lobby: pair everyone who is present, entered, and free.
async fn arena_pass(
    state: &AppState,
    id: Uuid,
    ends_at: OffsetDateTime,
    config: &GameConfig,
) -> Result<Pass, crate::auth::AuthError> {
    if OffsetDateTime::now_utc() >= ends_at {
        return Ok(Pass::Over);
    }

    // Redis first, and nothing else unless it's worth it: with fewer than
    // two people waiting there is no pairing to make and no reason to touch
    // Postgres at all.
    let present: HashSet<Uuid> = state.redis_client.lobby_present(id).await.into_iter().collect();
    if present.len() < 2 {
        return Ok(Pass::Quiet);
    }

    let store = &state.tournament_store;
    let Some(lock) = store.try_pairing_lock(id).await? else {
        // Another instance is pairing this arena right now.
        return Ok(Pass::Busy);
    };

    let players = store.players(id).await?;
    let waiting_users: Vec<Uuid> = players
        .iter()
        .filter(|p| !p.withdrawn)
        .filter_map(|p| p.user_id)
        .filter(|u| present.contains(u))
        .collect();
    // "Free" means not in *any* game, tournament or otherwise.
    let busy = state.game_store.active_games_for(&waiting_users).await?;
    let free: HashSet<Uuid> = players
        .iter()
        .filter(|p| p.user_id.is_some_and(|u| waiting_users.contains(&u) && !busy.contains_key(&u)))
        .map(|p| p.id)
        .collect();
    if free.len() < 2 {
        lock.commit().await?;
        return Ok(Pass::Quiet);
    }

    let pairings = store.pairings(id).await?;
    let entrants: Vec<_> = players.iter().map(PlayerRow::entrant).collect();
    let records: Vec<_> = pairings.iter().map(PairingRow::record).collect();
    let waiting: Vec<_> = tally(&entrants, &records)
        .into_iter()
        .filter(|t| free.contains(&t.id))
        .collect();

    let user_of: HashMap<Uuid, Uuid> = players.iter().filter_map(|p| Some((p.id, p.user_id?))).collect();
    for (white, black) in arena_pairings(&waiting) {
        let (Some(&white_user), Some(&black_user)) = (user_of.get(&white), user_of.get(&black)) else {
            continue;
        };
        match state.create_tournament_game(config.clone(), white_user, black_user, id).await {
            Ok(game_id) => {
                if let Err(e) = store.insert_arena_pairing(id, white, black, game_id).await {
                    tracing::error!(?e, %game_id, "failed to record arena pairing");
                }
            }
            Err(e) => tracing::warn!(?e, tournament_id = %id, "failed to create arena game"),
        }
    }

    lock.commit().await?;
    Ok(Pass::Busy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::JoinOutcome;
    use fred::prelude::*;
    use leptos::config::LeptosOptions;
    use shakmaty::KnownOutcome;
    use shared::messages::GameOverReason;
    use shared::{NewTournament, PairingResult};
    use sqlx::PgPool;

    async fn make_app_state(pool: PgPool) -> AppState {
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let config = Config::from_url(&redis_url).expect("invalid REDIS_URL");
        let redis_pool = Pool::new(config.clone(), None, None, None, 2).expect("build pool");
        redis_pool.connect();
        redis_pool.wait_for_connect().await.expect("Redis connect");
        let subscriber = fred::clients::SubscriberClient::new(config, None, None, None);
        let leptos_options = LeptosOptions::builder().output_name("test").build();
        AppState::new(leptos_options, pool, redis_pool, subscriber).await
    }

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

    fn new_tournament(format: TournamentFormat, venue: TournamentVenue, rounds: u32) -> NewTournament {
        NewTournament {
            name: "Test event".into(),
            format,
            venue,
            initial_seconds: 180,
            increment_seconds: 0,
            rated: false,
            starts_in_minutes: 0,
            duration_minutes: 30,
            rounds,
        }
    }

    async fn row(state: &AppState, id: Uuid) -> TournamentRow {
        state.tournament_store.get(id, None).await.unwrap().unwrap()
    }

    /// Ends a live tournament game the way the websocket handler does:
    /// through the room, then the store.
    async fn end_game(state: &AppState, game_id: Uuid, outcome: KnownOutcome, reason: GameOverReason) {
        let room = state.get_game_room(&game_id).await.expect("room exists");
        let plan = room.lock().await.end_game(outcome, reason);
        crate::db::finalize_now(&state.game_store, &state.redis_client, plan).await;
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn online_swiss_plays_itself_through_to_a_finish(pool: PgPool) {
        let state = make_app_state(pool.clone()).await;
        let organizer = insert_user(&pool, "organizer").await;
        let id = state
            .tournament_store
            .create(
                organizer,
                &new_tournament(TournamentFormat::Swiss, TournamentVenue::Online, 2),
                OffsetDateTime::now_utc(),
            )
            .await
            .unwrap();
        for name in ["alice", "bobby", "carol", "david"] {
            let user = insert_user(&pool, name).await;
            state.tournament_store.join(id, user).await.unwrap();
        }

        start(&state, &row(&state, id).await).await.unwrap();
        let live = row(&state, id).await;
        assert_eq!(live.status(), TournamentStatus::Live);
        assert_eq!(live.current_round, 1);

        let round_one = state.tournament_store.pairings(id).await.unwrap();
        assert_eq!(round_one.len(), 2);
        let games: Vec<Uuid> = round_one.iter().map(|p| p.game_id.expect("online pairings get a game")).collect();
        for game in &games {
            let room = state.get_game_room(game).await.expect("game room was created");
            assert_eq!(room.lock().await.tournament_id, Some(id));
        }

        // Round two can't be paired while round one is still being played.
        assert!(open_next_round(&state, &live).await.is_err());

        // One game is won; the other is aborted, which settles it as void.
        end_game(&state, games[0], KnownOutcome::Decisive { winner: shakmaty::Color::White }, GameOverReason::Resignation)
            .await;
        assert!(open_next_round(&state, &live).await.is_err(), "one game is still undecided");
        end_game(&state, games[1], KnownOutcome::Draw, GameOverReason::Abort).await;

        let settled = state.tournament_store.pairings(id).await.unwrap();
        let result_of = |game: Uuid| settled.iter().find(|p| p.game_id == Some(game)).unwrap().result();
        assert_eq!(result_of(games[0]), Some(PairingResult::White));
        assert_eq!(result_of(games[1]), Some(PairingResult::Void));

        // The organizer awards the aborted game, which is allowed now that
        // the game itself has reported.
        let void_pairing = settled.iter().find(|p| p.game_id == Some(games[1])).unwrap().id;
        assert!(state.tournament_store.set_result(id, void_pairing, Some(PairingResult::Black)).await.unwrap());

        open_next_round(&state, &live).await.unwrap();
        let after = row(&state, id).await;
        assert_eq!(after.current_round, 2);
        let round_two: Vec<_> = state
            .tournament_store
            .pairings(id)
            .await
            .unwrap()
            .into_iter()
            .filter(|p| p.round == 2)
            .collect();
        assert_eq!(round_two.len(), 2);

        // The two round-one winners meet, and nobody replays round one.
        let winners: HashSet<Uuid> = [
            settled.iter().find(|p| p.game_id == Some(games[0])).unwrap().white_player_id,
            settled.iter().find(|p| p.game_id == Some(games[1])).unwrap().black_player_id.unwrap(),
        ]
        .into();
        assert!(round_two.iter().any(|p| {
            winners.contains(&p.white_player_id) && p.black_player_id.is_some_and(|b| winners.contains(&b))
        }));

        for p in &round_two {
            end_game(&state, p.game_id.unwrap(), KnownOutcome::Draw, GameOverReason::DrawAgreement).await;
        }
        // Past the last round, "next round" is "finish".
        open_next_round(&state, &after).await.unwrap();
        assert_eq!(row(&state, id).await.status(), TournamentStatus::Finished);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_round_waits_for_players_who_are_in_another_game(pool: PgPool) {
        let state = make_app_state(pool.clone()).await;
        let organizer = insert_user(&pool, "organizer").await;
        let alice = insert_user(&pool, "alice").await;
        let bobby = insert_user(&pool, "bobby").await;
        let outsider = insert_user(&pool, "outsider").await;
        let id = state
            .tournament_store
            .create(
                organizer,
                &new_tournament(TournamentFormat::Swiss, TournamentVenue::Online, 1),
                OffsetDateTime::now_utc(),
            )
            .await
            .unwrap();
        state.tournament_store.join(id, alice).await.unwrap();
        state.tournament_store.join(id, bobby).await.unwrap();

        // Alice is mid-game against someone who isn't in the tournament.
        let other_game = state
            .create_game(game_config(&row(&state, id).await), alice, outsider, (0.0, 0.0))
            .await
            .unwrap();

        assert!(start(&state, &row(&state, id).await).await.is_err());
        // The tournament did go live; only the round is held back, so the
        // organizer just presses the button again.
        let live = row(&state, id).await;
        assert_eq!((live.status(), live.current_round), (TournamentStatus::Live, 0));
        assert!(state.tournament_store.pairings(id).await.unwrap().is_empty());

        end_game(&state, other_game, KnownOutcome::Draw, GameOverReason::DrawAgreement).await;
        open_next_round(&state, &live).await.unwrap();
        let pairings = state.tournament_store.pairings(id).await.unwrap();
        assert_eq!(pairings.len(), 1);
        state.redis_client.active_game_remove(pairings[0].game_id.unwrap()).await;
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn otb_round_robin_runs_on_organizer_results_alone(pool: PgPool) {
        let state = make_app_state(pool.clone()).await;
        let organizer = insert_user(&pool, "organizer").await;
        let id = state
            .tournament_store
            .create(
                organizer,
                &new_tournament(TournamentFormat::RoundRobin, TournamentVenue::Otb, 0),
                OffsetDateTime::now_utc(),
            )
            .await
            .unwrap();
        let mut players = Vec::new();
        for name in ["Ann", "Ben", "Cat"] {
            let JoinOutcome::Joined(p) = state.tournament_store.add_named_player(id, name).await.unwrap() else {
                panic!("full");
            };
            players.push(p);
        }

        start(&state, &row(&state, id).await).await.unwrap();
        // Three players: three rounds, each with one game and one bye.
        assert_eq!(row(&state, id).await.total_rounds, Some(3));

        for round in 1..=3 {
            let current = row(&state, id).await;
            assert_eq!(current.current_round, round);
            let pairings: Vec<_> = state
                .tournament_store
                .pairings(id)
                .await
                .unwrap()
                .into_iter()
                .filter(|p| p.round == round)
                .collect();
            assert_eq!(pairings.len(), 2);
            let game = pairings.iter().find(|p| p.black_player_id.is_some()).unwrap();
            assert!(game.game_id.is_none(), "over the board: no game on the site");

            assert!(open_next_round(&state, &current).await.is_err(), "result not entered yet");
            // White always wins.
            assert!(state.tournament_store.set_result(id, game.id, Some(PairingResult::White)).await.unwrap());
            open_next_round(&state, &current).await.unwrap();
        }
        assert_eq!(row(&state, id).await.status(), TournamentStatus::Finished);

        // Everyone played two games and had one bye; nobody met twice.
        let view = build_view(&state, id, None).await.unwrap();
        assert_eq!(view.standings.len(), 3);
        assert!(view.standings.iter().all(|s| s.played == 2));
        let total: u32 = view.standings.iter().map(|s| s.score2).sum();
        assert_eq!(total, 3 * 2 + 3 * 2, "three decisive games plus three byes");
        assert_eq!(view.standings[0].rank, 1);
        assert!(view.standings.windows(2).all(|w| w[0].score2 >= w[1].score2));
    }

    /// An invite can be sent once per friend per tournament, however many
    /// times the button is pressed.
    #[tokio::test]
    async fn a_tournament_invite_can_only_be_claimed_once() {
        let redis = crate::state::make_redis_client().await;
        let (tournament, from, to) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        assert!(redis.tournament_invite_claim(tournament, from, to).await);
        assert!(!redis.tournament_invite_claim(tournament, from, to).await);
        assert!(redis.tournament_invite_claim(tournament, from, Uuid::new_v4()).await, "other friends are separate");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_tie_for_the_podium_can_be_played_off_in_an_extra_round(pool: PgPool) {
        let state = make_app_state(pool.clone()).await;
        let organizer = insert_user(&pool, "organizer").await;
        let id = state
            .tournament_store
            .create(
                organizer,
                &new_tournament(TournamentFormat::Swiss, TournamentVenue::Otb, 1),
                OffsetDateTime::now_utc(),
            )
            .await
            .unwrap();
        let mut ids = Vec::new();
        for name in ["Ann", "Ben", "Cat", "Dan"] {
            let JoinOutcome::Joined(p) = state.tournament_store.add_named_player(id, name).await.unwrap() else {
                panic!("full");
            };
            ids.push(p);
        }
        start(&state, &row(&state, id).await).await.unwrap();

        let live = row(&state, id).await;
        assert!(open_tiebreak_round(&state, &live).await.is_err(), "round 1 is still undecided");

        // Both games drawn: everyone is level on a point each.
        let round: Vec<_> = state.tournament_store.pairings(id).await.unwrap();
        for p in &round {
            state.tournament_store.set_result(id, p.id, Some(PairingResult::Draw)).await.unwrap();
        }
        open_tiebreak_round(&state, &live).await.unwrap();
        let after = row(&state, id).await;
        assert_eq!((after.current_round, after.total_rounds), (2, Some(2)));
        let playoff: Vec<_> = state
            .tournament_store
            .pairings(id)
            .await
            .unwrap()
            .into_iter()
            .filter(|p| p.round == 2)
            .collect();
        assert_eq!(playoff.len(), 2, "all four are tied, so both games are played off");

        // A second click can't add another round on top.
        assert!(open_tiebreak_round(&state, &live).await.is_err());

        // One decisive game, one draw: first place is now clear, but the
        // drawn pair are still tied for second/third.
        state.tournament_store.set_result(id, playoff[0].id, Some(PairingResult::White)).await.unwrap();
        state.tournament_store.set_result(id, playoff[1].id, Some(PairingResult::Draw)).await.unwrap();
        let after = row(&state, id).await;
        open_tiebreak_round(&state, &after).await.unwrap();
        assert_eq!(row(&state, id).await.total_rounds, Some(3));

        // With the podium settled there is nothing to play off.
        for p in state.tournament_store.pairings(id).await.unwrap().into_iter().filter(|p| p.round == 3) {
            state.tournament_store.set_result(id, p.id, Some(PairingResult::White)).await.unwrap();
        }
        let after = row(&state, id).await;
        let view = build_view(&state, id, None).await.unwrap();
        let level = shared::tiebreak_group(
            &view.standings.iter().map(|r| (r.player_id, r.score2)).collect::<Vec<_>>(),
        );
        if level.is_empty() {
            assert!(open_tiebreak_round(&state, &after).await.is_err());
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn an_arena_pairs_only_the_players_who_are_present_and_free(pool: PgPool) {
        let state = make_app_state(pool.clone()).await;
        let now = OffsetDateTime::now_utc();
        state
            .tournament_store
            .ensure_scheduled(&[ScheduledSlot {
                key: "test-arena",
                name: "Test Arena",
                initial_seconds: 180,
                increment_seconds: 0,
                starts_at: now - time::Duration::minutes(1),
                duration_minutes: 30,
            }])
            .await
            .unwrap();
        state.tournament_store.sync_arena_status().await.unwrap();
        let arena = state.tournament_store.list(None).await.unwrap().remove(0);
        assert_eq!(arena.status(), TournamentStatus::Live);
        let id = arena.id;
        let ends_at = arena.starts_at + time::Duration::minutes(30);
        let config = game_config(&arena);

        let alice = insert_user(&pool, "alice").await;
        let bobby = insert_user(&pool, "bobby").await;
        let carol = insert_user(&pool, "carol").await;
        let lurker = insert_user(&pool, "lurker").await;
        for user in [alice, bobby, carol] {
            state.tournament_store.join(id, user).await.unwrap();
        }

        // Nobody on the page: nothing happens, and Postgres isn't asked.
        assert!(matches!(arena_pass(&state, id, ends_at, &config).await.unwrap(), Pass::Quiet));

        // Alice and Bobby are on the page. Carol entered but left; the
        // lurker is on the page but never entered.
        for user in [alice, bobby, lurker] {
            state.redis_client.lobby_touch(id, user).await;
        }
        assert!(matches!(arena_pass(&state, id, ends_at, &config).await.unwrap(), Pass::Busy));

        let pairings = state.tournament_store.pairings(id).await.unwrap();
        assert_eq!(pairings.len(), 1);
        let game_id = pairings[0].game_id.unwrap();
        let room = state.get_game_room(&game_id).await.unwrap();
        let (white, black) = {
            let gr = room.lock().await;
            (gr.game.white_player, gr.game.black_player)
        };
        assert_eq!(HashSet::from([white, black]), HashSet::from([alice, bobby]));
        assert!(room.lock().await.game.config.rated.is_rated(), "scheduled arenas are rated");

        // Still on the page, but now playing: not paired a second time.
        assert!(matches!(arena_pass(&state, id, ends_at, &config).await.unwrap(), Pass::Quiet));
        assert_eq!(state.tournament_store.pairings(id).await.unwrap().len(), 1);

        // The game ends; both are free again and get the return game with
        // colours reversed.
        end_game(&state, game_id, KnownOutcome::Decisive { winner: shakmaty::Color::White }, GameOverReason::Checkmate)
            .await;
        assert!(matches!(arena_pass(&state, id, ends_at, &config).await.unwrap(), Pass::Busy));
        let pairings = state.tournament_store.pairings(id).await.unwrap();
        assert_eq!(pairings.len(), 2);
        let rematch = state.get_game_room(&pairings[1].game_id.unwrap()).await.unwrap();
        assert_eq!(rematch.lock().await.game.white_player, black);

        // Once the window closes the loop is told to stop.
        let closed = now - time::Duration::seconds(1);
        assert!(matches!(arena_pass(&state, id, closed, &config).await.unwrap(), Pass::Over));

        let view = build_view(&state, id, None).await.unwrap();
        assert_eq!(view.standings[0].score2, 2);
        assert_eq!(view.standings[0].username.as_deref(), if white == alice { Some("alice") } else { Some("bobby") });

        state.redis_client.active_game_remove(pairings[1].game_id.unwrap()).await;
        for user in [alice, bobby, lurker] {
            state.redis_client.lobby_leave(id, user).await;
        }
    }
}
