//! Tournament types and the pure half of running one: scoring, standings and
//! pairing.
//!
//! Everything that decides *who plays whom* and *who is winning* lives here
//! rather than next to the database code, for the same reason username rules
//! live in `shared`: it is plain arithmetic over ids, it has no business
//! knowing about Postgres, and it is the part most worth unit-testing.
//!
//! Scores are carried as **half-points** (`score2`: a win is 2, a draw 1)
//! so that standings are integer comparisons all the way down. Format with
//! [`format_score2`] at the edge.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TournamentFormat {
    /// Fixed time window, continuous pairing — play as many games as you can.
    Arena,
    /// Fixed number of rounds, each pairing players on similar scores.
    Swiss,
    /// Everyone plays everyone once.
    RoundRobin,
}

impl TournamentFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Arena => "arena",
            Self::Swiss => "swiss",
            Self::RoundRobin => "round_robin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "arena" => Some(Self::Arena),
            "swiss" => Some(Self::Swiss),
            "round_robin" => Some(Self::RoundRobin),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Arena => "Arena",
            Self::Swiss => "Swiss",
            Self::RoundRobin => "Round robin",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TournamentVenue {
    /// Games are played on the site; results record themselves.
    Online,
    /// Over the board. The site keeps pairings and scores; the organizer
    /// enters each result.
    Otb,
}

impl TournamentVenue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Otb => "otb",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "online" => Some(Self::Online),
            "otb" => Some(Self::Otb),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Otb => "Over the board",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TournamentStatus {
    Upcoming,
    Live,
    Finished,
    Cancelled,
}

impl TournamentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Upcoming => "upcoming",
            Self::Live => "live",
            Self::Finished => "finished",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "upcoming" => Some(Self::Upcoming),
            "live" => Some(Self::Live),
            "finished" => Some(Self::Finished),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_over(&self) -> bool {
        matches!(self, Self::Finished | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PairingResult {
    White,
    Black,
    Draw,
    /// No opponent this round; scores as a win.
    Bye,
    /// The game was aborted. Nobody scores and it is not a played game.
    Void,
}

impl PairingResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Black => "black",
            Self::Draw => "draw",
            Self::Bye => "bye",
            Self::Void => "void",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "white" => Some(Self::White),
            "black" => Some(Self::Black),
            "draw" => Some(Self::Draw),
            "bye" => Some(Self::Bye),
            "void" => Some(Self::Void),
            _ => None,
        }
    }

    /// The conventional score-sheet spelling.
    pub fn label(&self) -> &'static str {
        match self {
            Self::White => "1 – 0",
            Self::Black => "0 – 1",
            Self::Draw => "½ – ½",
            Self::Bye => "bye",
            Self::Void => "aborted",
        }
    }
}

/// `"3"`, `"3½"`, `"½"`, `"0"`.
pub fn format_score2(score2: u32) -> String {
    match (score2 / 2, score2 % 2) {
        (0, 1) => "½".to_string(),
        (whole, 1) => format!("{whole}½"),
        (whole, _) => whole.to_string(),
    }
}

/// `"5+3"` for 300s + 3s; sub-minute and uneven base times spell out.
pub fn format_time_control(initial_seconds: i32, increment_seconds: i32) -> String {
    let base = if initial_seconds % 60 == 0 {
        (initial_seconds / 60).to_string()
    } else if initial_seconds < 60 {
        format!("{initial_seconds}s")
    } else {
        format!("{}m{}s", initial_seconds / 60, initial_seconds % 60)
    };
    format!("{base}+{increment_seconds}")
}

// ---------------------------------------------------------------------------
// Limits. Shared so the create form and the server fn reject the same things.
// ---------------------------------------------------------------------------

pub const TOURNAMENT_NAME_MIN_LEN: usize = 3;
pub const TOURNAMENT_NAME_MAX_LEN: usize = 60;
pub const TOURNAMENT_MAX_PLAYERS: usize = 200;
/// Everyone-plays-everyone grows quadratically; past this it stops being an
/// evening and starts being a league.
pub const ROUND_ROBIN_MAX_PLAYERS: usize = 16;
pub const SWISS_MAX_ROUNDS: u32 = 15;
pub const ARENA_MIN_MINUTES: u32 = 10;
pub const ARENA_MAX_MINUTES: u32 = 180;
/// How far ahead a user-created arena may be scheduled.
pub const ARENA_MAX_LEAD_MINUTES: u32 = 7 * 24 * 60;
pub const PLAYER_NAME_MAX_LEN: usize = 40;

/// What the create form submits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTournament {
    pub name: String,
    pub format: TournamentFormat,
    pub venue: TournamentVenue,
    pub initial_seconds: u32,
    pub increment_seconds: u32,
    pub rated: bool,
    /// Arena: minutes from now until it opens. Ignored otherwise — a swiss
    /// or round robin starts when its organizer starts it.
    pub starts_in_minutes: u32,
    /// Arena only.
    pub duration_minutes: u32,
    /// Swiss only. A round robin's length follows from its field.
    pub rounds: u32,
}

impl NewTournament {
    /// `Ok(())` if this describes a tournament the site can run, otherwise a
    /// message fit to show the organizer directly.
    pub fn validate(&self) -> Result<(), &'static str> {
        let name_len = self.name.trim().chars().count();
        if name_len < TOURNAMENT_NAME_MIN_LEN {
            return Err("Give the tournament a name of at least 3 characters");
        }
        if name_len > TOURNAMENT_NAME_MAX_LEN {
            return Err("Tournament name must be at most 60 characters");
        }
        if self.format == TournamentFormat::Arena && self.venue == TournamentVenue::Otb {
            return Err("Arenas are online only — pick swiss or round robin for over the board");
        }
        // Rating is for the official daily arenas only. Anyone can create a
        // tournament and enter their own alts into it, so letting those move
        // ratings would make rating-farming a button press.
        if self.rated {
            return Err("Only the official daily tournaments are rated");
        }
        if !(30..=3 * 60 * 60).contains(&self.initial_seconds) {
            return Err("Base time must be between 30 seconds and 3 hours");
        }
        if self.increment_seconds > 180 {
            return Err("Increment must be at most 180 seconds");
        }
        match self.format {
            TournamentFormat::Arena => {
                if !(ARENA_MIN_MINUTES..=ARENA_MAX_MINUTES).contains(&self.duration_minutes) {
                    return Err("An arena runs for between 10 minutes and 3 hours");
                }
                if self.starts_in_minutes > ARENA_MAX_LEAD_MINUTES {
                    return Err("An arena can be scheduled at most a week ahead");
                }
            }
            TournamentFormat::Swiss => {
                if !(1..=SWISS_MAX_ROUNDS).contains(&self.rounds) {
                    return Err("A swiss runs for between 1 and 15 rounds");
                }
            }
            TournamentFormat::RoundRobin => {}
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Wire types for the tournament pages.
// ---------------------------------------------------------------------------

/// The players who should play off before the podium is final, if any.
///
/// `ranked` is every non-withdrawn player best-first as `(id, score2)`. A tie
/// matters only if it touches the top three places: the first group of two or
/// more players on the same score that *starts* inside them is the one that
/// needs a decider (a tie for first, or for second/third, or for third).
/// Returns that group in rank order, or nothing when the podium is already
/// unambiguous.
pub fn tiebreak_group(ranked: &[(Uuid, u32)]) -> Vec<Uuid> {
    let mut start = 0;
    while start < ranked.len() && start < 3 {
        let score = ranked[start].1;
        let end = ranked[start..].iter().position(|(_, s)| *s != score).map_or(ranked.len(), |n| start + n);
        if end - start >= 2 {
            return ranked[start..end].iter().map(|(id, _)| *id).collect();
        }
        start = end;
    }
    Vec::new()
}

/// One row of the tournament list, and the header of a tournament page.
/// Timestamps are UNIX seconds: `shared` deliberately has no date library,
/// and the client only ever needs to subtract them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TournamentSummary {
    pub id: Uuid,
    pub name: String,
    pub format: TournamentFormat,
    pub venue: TournamentVenue,
    pub status: TournamentStatus,
    pub initial_seconds: i32,
    pub increment_seconds: i32,
    pub rated: bool,
    pub starts_at: i64,
    pub duration_minutes: Option<i32>,
    pub total_rounds: Option<i32>,
    pub current_round: i32,
    pub player_count: u32,
    /// `None` for the site's own scheduled events.
    pub organizer: Option<String>,
    pub official: bool,
    /// The viewer organizes this or is entered in it.
    pub mine: bool,
}

impl TournamentSummary {
    /// When an arena closes. `None` for round-based formats, which end when
    /// their last round does.
    pub fn ends_at(&self) -> Option<i64> {
        self.duration_minutes.map(|m| self.starts_at + i64::from(m) * 60)
    }

    pub fn time_control_label(&self) -> String {
        format_time_control(self.initial_seconds, self.increment_seconds)
    }

    /// Bullet / blitz / rapid / classical, by the same rule that decides
    /// which rating a game moves.
    pub fn speed(&self) -> crate::Category {
        crate::TimeControl {
            initial_time: i64::from(self.initial_seconds) * 1000,
            mode: crate::TimeMode::Increment(i64::from(self.increment_seconds) * 1000),
        }
        .category()
    }
}

/// What the tournament list is narrowed to. Every field left at its default
/// matches everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TournamentFilter {
    /// Case-insensitive, matched against the name and the organizer.
    pub query: String,
    pub venue: Option<TournamentVenue>,
    pub format: Option<TournamentFormat>,
    pub speed: Option<crate::Category>,
    pub rated: Option<bool>,
}

impl TournamentFilter {
    pub fn is_active(&self) -> bool {
        !self.query.trim().is_empty()
            || self.venue.is_some()
            || self.format.is_some()
            || self.speed.is_some()
            || self.rated.is_some()
    }

    pub fn matches(&self, t: &TournamentSummary) -> bool {
        let query = self.query.trim().to_lowercase();
        (query.is_empty()
            || t.name.to_lowercase().contains(&query)
            || t.organizer.as_ref().is_some_and(|o| o.to_lowercase().contains(&query)))
            && self.venue.is_none_or(|v| v == t.venue)
            && self.format.is_none_or(|f| f == t.format)
            && self.speed.is_none_or(|s| s == t.speed())
            && self.rated.is_none_or(|r| r == t.rated)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingRow {
    pub rank: u32,
    pub player_id: Uuid,
    /// `None` for an over-the-board entrant without an account.
    pub username: Option<String>,
    pub name: String,
    pub withdrawn: bool,
    pub score2: u32,
    pub played: u32,
    pub wins: u32,
    pub draws: u32,
    pub losses: u32,
    /// Buchholz, in half-points.
    pub tiebreak2: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingView {
    pub id: Uuid,
    pub round: i32,
    pub white_id: Uuid,
    pub white_name: String,
    pub black_id: Option<Uuid>,
    pub black_name: Option<String>,
    pub result: Option<PairingResult>,
    pub game_id: Option<Uuid>,
}

/// What the person looking at the page may do, decided server-side.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TournamentViewer {
    pub signed_in: bool,
    /// A full account with a username — guests can watch but not enter.
    pub can_enter: bool,
    pub player_id: Option<Uuid>,
    pub withdrawn: bool,
    pub is_organizer: bool,
    /// A tournament game of theirs that is in progress right now. The page
    /// sends them to it.
    pub active_game: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TournamentView {
    pub summary: TournamentSummary,
    pub standings: Vec<StandingRow>,
    /// Round-based formats: every pairing. Arena: the most recent games.
    pub pairings: Vec<PairingView>,
    pub viewer: TournamentViewer,
    /// The server's clock when this was built. Countdowns measure from here
    /// rather than from the browser's clock, which may simply be wrong.
    pub server_now: i64,
}

// ---------------------------------------------------------------------------
// Standings.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entrant {
    pub id: Uuid,
    pub seed: i32,
    pub withdrawn: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairingRecord {
    pub round: i32,
    pub white: Uuid,
    pub black: Option<Uuid>,
    pub result: Option<PairingResult>,
}

/// One entrant's record so far. [`tally`] returns these in ranking order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tally {
    pub id: Uuid,
    pub seed: i32,
    pub withdrawn: bool,
    pub score2: u32,
    pub played: u32,
    pub wins: u32,
    pub draws: u32,
    pub losses: u32,
    pub tiebreak2: u32,
    pub whites: u32,
    pub blacks: u32,
    pub had_bye: bool,
    /// Everyone they have been paired against, decided or not, in order.
    pub opponents: Vec<Uuid>,
    /// `Some(true)` if they had white in their most recent game.
    pub last_was_white: Option<bool>,
}

impl Tally {
    fn new(e: &Entrant) -> Self {
        Tally {
            id: e.id,
            seed: e.seed,
            withdrawn: e.withdrawn,
            score2: 0,
            played: 0,
            wins: 0,
            draws: 0,
            losses: 0,
            tiebreak2: 0,
            whites: 0,
            blacks: 0,
            had_bye: false,
            opponents: Vec::new(),
            last_was_white: None,
        }
    }

    /// Positive when they have had white more often than black.
    fn color_balance(&self) -> i64 {
        i64::from(self.whites) - i64::from(self.blacks)
    }
}

/// Folds every pairing into a per-entrant record and ranks the field.
///
/// Ranking is score, then Buchholz (the summed scores of the opponents you
/// actually played — beating strong opposition outranks beating weak), then
/// wins, then join order so the result is total and stable.
///
/// `pairings` must be in the order they were created; colour history and
/// "most recent opponent" both read it as a timeline.
pub fn tally(entrants: &[Entrant], pairings: &[PairingRecord]) -> Vec<Tally> {
    let mut by_id: HashMap<Uuid, Tally> = entrants.iter().map(|e| (e.id, Tally::new(e))).collect();

    for p in pairings {
        let Some(black) = p.black else {
            if let Some(t) = by_id.get_mut(&p.white) {
                t.had_bye = true;
                if p.result == Some(PairingResult::Bye) {
                    t.score2 += 2;
                }
            }
            continue;
        };

        // An aborted game leaves no trace: it wasn't played, it doesn't count
        // toward colours, and the two are free to be paired again.
        if p.result == Some(PairingResult::Void) {
            continue;
        }

        if let Some(t) = by_id.get_mut(&p.white) {
            t.opponents.push(black);
            t.whites += 1;
            t.last_was_white = Some(true);
        }
        if let Some(t) = by_id.get_mut(&black) {
            t.opponents.push(p.white);
            t.blacks += 1;
            t.last_was_white = Some(false);
        }

        let (white_pts, black_pts) = match p.result {
            Some(PairingResult::White) => (2, 0),
            Some(PairingResult::Black) => (0, 2),
            Some(PairingResult::Draw) => (1, 1),
            _ => continue,
        };
        for (id, pts) in [(p.white, white_pts), (black, black_pts)] {
            if let Some(t) = by_id.get_mut(&id) {
                t.played += 1;
                t.score2 += pts;
                match pts {
                    2 => t.wins += 1,
                    1 => t.draws += 1,
                    _ => t.losses += 1,
                }
            }
        }
    }

    // Buchholz needs everyone's final score, so it is a second pass.
    let scores: HashMap<Uuid, u32> = by_id.iter().map(|(id, t)| (*id, t.score2)).collect();
    for p in pairings {
        let Some(black) = p.black else { continue };
        if !matches!(
            p.result,
            Some(PairingResult::White | PairingResult::Black | PairingResult::Draw)
        ) {
            continue;
        }
        if let Some(t) = by_id.get_mut(&p.white) {
            t.tiebreak2 += scores.get(&black).copied().unwrap_or(0);
        }
        if let Some(t) = by_id.get_mut(&black) {
            t.tiebreak2 += scores.get(&p.white).copied().unwrap_or(0);
        }
    }

    let mut ranked: Vec<Tally> = by_id.into_values().collect();
    ranked.sort_by(|a, b| {
        b.score2
            .cmp(&a.score2)
            .then(b.tiebreak2.cmp(&a.tiebreak2))
            .then(b.wins.cmp(&a.wins))
            .then(a.seed.cmp(&b.seed))
    });
    ranked
}

// ---------------------------------------------------------------------------
// Pairing.
// ---------------------------------------------------------------------------

/// `(white, black)`; `black` is `None` for a bye.
pub type Pairing = (Uuid, Option<Uuid>);

/// Who takes white when `a` (the higher-ranked) meets `b`.
///
/// Whoever has had white less often gets it. On a tie, whoever just had
/// black. Still tied (round one, typically), `tiebreak_a_white` decides —
/// callers alternate it down the boards so the top half doesn't all get the
/// same colour.
fn assign_colors(a: &Tally, b: &Tally, tiebreak_a_white: bool) -> (Uuid, Uuid) {
    let a_white = match a.color_balance().cmp(&b.color_balance()) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => match (a.last_was_white, b.last_was_white) {
            (Some(false), Some(true)) => true,
            (Some(true), Some(false)) => false,
            _ => tiebreak_a_white,
        },
    };
    if a_white { (a.id, b.id) } else { (b.id, a.id) }
}

/// Upper bound on how hard [`swiss_pairings`] searches for a rematch-free
/// round before settling. The search is a backtracking one and is instant
/// for any realistic field; this only exists so a pathological late round
/// can't turn a button click into a stall.
const SWISS_SEARCH_BUDGET: u32 = 50_000;

/// Pairs the next swiss round.
///
/// `ranked` is [`tally`]'s output. Withdrawn players are skipped. With an
/// odd field the bye goes to the lowest-ranked player who hasn't had one.
/// The rest are paired top-down, each against the highest-ranked player
/// they haven't met; if that paints the bottom of the field into a corner
/// the search backtracks. Only when no rematch-free round exists at all
/// (more rounds than the field can support) are rematches allowed.
pub fn swiss_pairings(ranked: &[Tally]) -> Vec<Pairing> {
    let mut pool: Vec<&Tally> = ranked.iter().filter(|t| !t.withdrawn).collect();
    let mut out: Vec<Pairing> = Vec::new();

    let bye = if pool.len() % 2 == 1 {
        let idx = pool
            .iter()
            .rposition(|t| !t.had_bye)
            .unwrap_or(pool.len() - 1);
        Some(pool.remove(idx).id)
    } else {
        None
    };

    let met: Vec<HashSet<Uuid>> = pool
        .iter()
        .map(|t| t.opponents.iter().copied().collect())
        .collect();

    fn search(
        remaining: &[usize],
        met: &[HashSet<Uuid>],
        pool: &[&Tally],
        budget: &mut u32,
        acc: &mut Vec<(usize, usize)>,
    ) -> bool {
        let Some((&first, rest)) = remaining.split_first() else {
            return true;
        };
        for (i, &cand) in rest.iter().enumerate() {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            if met[first].contains(&pool[cand].id) {
                continue;
            }
            let mut next: Vec<usize> = rest.to_vec();
            next.remove(i);
            acc.push((first, cand));
            if search(&next, met, pool, budget, acc) {
                return true;
            }
            acc.pop();
        }
        false
    }

    let indices: Vec<usize> = (0..pool.len()).collect();
    let mut pairs: Vec<(usize, usize)> = Vec::with_capacity(pool.len() / 2);
    let mut budget = SWISS_SEARCH_BUDGET;
    if !search(&indices, &met, &pool, &mut budget, &mut pairs) {
        pairs = indices.chunks(2).map(|c| (c[0], c[1])).collect();
    }

    for (board, (a, b)) in pairs.into_iter().enumerate() {
        let (white, black) = assign_colors(pool[a], pool[b], board % 2 == 0);
        out.push((white, Some(black)));
    }
    if let Some(id) = bye {
        out.push((id, None));
    }
    out
}

/// How many rounds a round robin of `players` needs: everyone meets everyone
/// once, and with an odd field each player also sits one round out.
pub fn round_robin_rounds(players: usize) -> u32 {
    match players {
        0 | 1 => 0,
        n if n % 2 == 0 => (n - 1) as u32,
        n => n as u32,
    }
}

/// Round `round` (1-based) of a round robin, by the circle method: seat one
/// player, rotate everyone else around them one place per round.
///
/// `entrants` must be the *whole* starting field in seed order, withdrawn
/// players included — the schedule is fixed when the tournament starts, and
/// dropping someone from the list would reshuffle every remaining round.
/// A withdrawn player's opponent gets the bye instead.
pub fn round_robin_round(entrants: &[Entrant], round: u32) -> Vec<Pairing> {
    let mut seats: Vec<Option<&Entrant>> = entrants.iter().map(Some).collect();
    if seats.len() % 2 == 1 {
        seats.push(None);
    }
    let n = seats.len();
    if n < 2 || round == 0 {
        return Vec::new();
    }

    let shift = ((round - 1) as usize) % (n - 1);
    let mut circle: Vec<Option<&Entrant>> = Vec::with_capacity(n);
    circle.push(seats[0]);
    for i in 0..n - 1 {
        circle.push(seats[1 + (i + n - 1 - shift) % (n - 1)]);
    }

    let mut out = Vec::new();
    for i in 0..n / 2 {
        let a = circle[i].filter(|e| !e.withdrawn);
        let b = circle[n - 1 - i].filter(|e| !e.withdrawn);
        match (a, b) {
            (Some(a), Some(b)) => {
                // The seated player alternates colour each round; the other
                // boards alternate down the table. Together that gives every
                // player a balanced split across the event.
                let a_white = if i == 0 { round % 2 == 1 } else { i % 2 == 0 };
                if a_white {
                    out.push((a.id, Some(b.id)));
                } else {
                    out.push((b.id, Some(a.id)));
                }
            }
            (Some(solo), None) | (None, Some(solo)) => out.push((solo.id, None)),
            (None, None) => {}
        }
    }
    out
}

/// Pairs whoever is waiting in an arena right now.
///
/// `waiting` is the slice of [`tally`]'s output that is present and free, in
/// ranking order. Neighbours in the standings play each other, skipping an
/// immediate rematch when anyone else is available — but two people alone
/// in an arena should still get to keep playing, so a rematch beats waiting.
/// With an odd number the last player waits for the next pass.
pub fn arena_pairings(waiting: &[Tally]) -> Vec<(Uuid, Uuid)> {
    let mut pool: Vec<&Tally> = waiting.iter().filter(|t| !t.withdrawn).collect();
    let mut out = Vec::new();

    while pool.len() >= 2 {
        let a = pool.remove(0);
        let just_played = |x: &Tally, y: &Tally| x.opponents.last() == Some(&y.id);
        let pick = pool
            .iter()
            .position(|b| !just_played(a, b) && !just_played(b, a))
            .unwrap_or(0);
        let b = pool.remove(pick);
        out.push(assign_colors(a, b, (a.whites + a.blacks).is_multiple_of(2)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_narrow_the_list() {
        let t = |name: &str, venue, format, initial_seconds, rated| TournamentSummary {
            id: Uuid::nil(),
            name: name.into(),
            format,
            venue,
            status: TournamentStatus::Upcoming,
            initial_seconds,
            increment_seconds: 0,
            rated,
            starts_at: 0,
            duration_minutes: None,
            total_rounds: None,
            current_round: 0,
            player_count: 0,
            organizer: Some("Magnus".into()),
            official: false,
            mine: false,
        };
        let arena = t("Daily Blitz Arena", TournamentVenue::Online, TournamentFormat::Arena, 180, true);
        let club = t("Tuesday club night", TournamentVenue::Otb, TournamentFormat::Swiss, 900, false);

        let none = TournamentFilter::default();
        assert!(!none.is_active());
        assert!(none.matches(&arena) && none.matches(&club));

        let by = |f: TournamentFilter| (f.matches(&arena), f.matches(&club));
        assert_eq!(by(TournamentFilter { query: " BLITZ ".into(), ..none.clone() }), (true, false));
        assert_eq!(by(TournamentFilter { query: "magnus".into(), ..none.clone() }), (true, true), "organizer matches too");
        assert_eq!(by(TournamentFilter { venue: Some(TournamentVenue::Otb), ..none.clone() }), (false, true));
        assert_eq!(by(TournamentFilter { format: Some(TournamentFormat::Arena), ..none.clone() }), (true, false));
        assert_eq!(by(TournamentFilter { speed: Some(crate::Category::Blitz), ..none.clone() }), (true, false));
        assert_eq!(by(TournamentFilter { speed: Some(crate::Category::Rapid), ..none.clone() }), (false, true));
        assert_eq!(by(TournamentFilter { rated: Some(true), ..none.clone() }), (true, false));
        assert!(TournamentFilter { rated: Some(false), ..none.clone() }.is_active());
        assert!(!TournamentFilter { query: "  ".into(), ..none }.is_active(), "blank search is no filter");
    }

    #[test]
    fn tiebreak_group_finds_ties_that_touch_the_podium() {
        let r = |scores: &[u32]| -> Vec<(Uuid, u32)> {
            scores.iter().enumerate().map(|(i, s)| (Uuid::from_u128(i as u128 + 1), *s)).collect()
        };
        let ids = |range: std::ops::Range<u128>| -> Vec<Uuid> { range.map(|i| Uuid::from_u128(i + 1)).collect() };
        assert!(tiebreak_group(&r(&[8, 6, 4, 2, 1])).is_empty(), "a clear podium needs no playoff");
        assert_eq!(tiebreak_group(&r(&[8, 6, 4, 4, 2])), ids(2..4));
        assert_eq!(tiebreak_group(&r(&[8, 8, 6])), ids(0..2));
        assert_eq!(tiebreak_group(&r(&[8, 6, 6, 4])), ids(1..3));
        assert_eq!(tiebreak_group(&r(&[8, 8, 8, 8])), ids(0..4));
        assert!(tiebreak_group(&r(&[8, 6, 4, 2, 2])).is_empty(), "a tie below the podium is nobody's business");
        assert!(tiebreak_group(&r(&[])).is_empty());
        assert!(tiebreak_group(&r(&[5])).is_empty());
    }

    fn entrants(n: usize) -> Vec<Entrant> {
        (0..n)
            .map(|i| Entrant { id: Uuid::from_u128(i as u128 + 1), seed: i as i32, withdrawn: false })
            .collect()
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn game(round: i32, white: u128, black: u128, result: PairingResult) -> PairingRecord {
        PairingRecord { round, white: id(white), black: Some(id(black)), result: Some(result) }
    }

    #[test]
    fn score_formatting() {
        assert_eq!(format_score2(0), "0");
        assert_eq!(format_score2(1), "½");
        assert_eq!(format_score2(2), "1");
        assert_eq!(format_score2(7), "3½");
    }

    #[test]
    fn time_control_formatting() {
        assert_eq!(format_time_control(300, 3), "5+3");
        assert_eq!(format_time_control(60, 0), "1+0");
        assert_eq!(format_time_control(30, 0), "30s+0");
        assert_eq!(format_time_control(90, 1), "1m30s+1");
    }

    #[test]
    fn tally_scores_wins_draws_and_byes() {
        let es = entrants(3);
        let ps = vec![
            game(1, 1, 2, PairingResult::White),
            PairingRecord { round: 1, white: id(3), black: None, result: Some(PairingResult::Bye) },
            game(2, 3, 1, PairingResult::Draw),
        ];
        let t = tally(&es, &ps);
        let by = |n: u128| t.iter().find(|x| x.id == id(n)).unwrap();

        assert_eq!(by(1).score2, 3);
        assert_eq!((by(1).wins, by(1).draws, by(1).losses, by(1).played), (1, 1, 0, 2));
        assert_eq!(by(2).score2, 0);
        assert_eq!(by(2).losses, 1);
        // A bye scores a full point but is not a played game.
        assert_eq!(by(3).score2, 3);
        assert_eq!(by(3).played, 1);
        assert!(by(3).had_bye);
    }

    #[test]
    fn void_games_score_nothing_and_leave_no_history() {
        let es = entrants(2);
        let ps = vec![game(0, 1, 2, PairingResult::Void)];
        let t = tally(&es, &ps);
        assert!(t.iter().all(|x| x.score2 == 0 && x.played == 0));
        assert!(t.iter().all(|x| x.opponents.is_empty() && x.whites + x.blacks == 0));
    }

    #[test]
    fn undecided_games_count_as_met_but_not_played() {
        let es = entrants(2);
        let ps = vec![PairingRecord { round: 1, white: id(1), black: Some(id(2)), result: None }];
        let t = tally(&es, &ps);
        assert!(t.iter().all(|x| x.played == 0 && x.opponents.len() == 1));
    }

    #[test]
    fn buchholz_breaks_a_tie_on_points() {
        // 1 and 2 both finish on one win. 1 beat player 3, who went on to
        // win a game; 2 beat player 4, who lost everything. Same score,
        // stronger opposition — 1 ranks first.
        let es = entrants(5);
        let ps = vec![
            game(1, 1, 3, PairingResult::White),
            game(1, 2, 4, PairingResult::White),
            game(2, 3, 5, PairingResult::White),
        ];
        let t = tally(&es, &ps);
        let pos = |n: u128| t.iter().position(|x| x.id == id(n)).unwrap();
        assert_eq!(t[pos(1)].score2, t[pos(2)].score2);
        assert!(t[pos(1)].tiebreak2 > t[pos(2)].tiebreak2);
        assert!(pos(1) < pos(2));
    }

    #[test]
    fn ranking_falls_back_to_seed_so_it_is_stable() {
        let t = tally(&entrants(4), &[]);
        let seeds: Vec<i32> = t.iter().map(|x| x.seed).collect();
        assert_eq!(seeds, vec![0, 1, 2, 3]);
    }

    fn assert_everyone_paired_once(pairs: &[Pairing], n: usize) {
        let mut seen = HashSet::new();
        for (w, b) in pairs {
            assert!(seen.insert(*w), "{w} paired twice");
            if let Some(b) = b {
                assert!(seen.insert(*b), "{b} paired twice");
            }
        }
        assert_eq!(seen.len(), n);
    }

    #[test]
    fn swiss_round_one_pairs_everyone() {
        let t = tally(&entrants(8), &[]);
        let pairs = swiss_pairings(&t);
        assert_eq!(pairs.len(), 4);
        assert!(pairs.iter().all(|(_, b)| b.is_some()));
        assert_everyone_paired_once(&pairs, 8);
    }

    #[test]
    fn swiss_odd_field_gives_exactly_one_bye_to_the_bottom() {
        let t = tally(&entrants(5), &[]);
        let pairs = swiss_pairings(&t);
        let byes: Vec<_> = pairs.iter().filter(|(_, b)| b.is_none()).collect();
        assert_eq!(byes.len(), 1);
        assert_eq!(byes[0].0, id(5), "lowest seed sits out round one");
        assert_everyone_paired_once(&pairs, 5);
    }

    #[test]
    fn swiss_does_not_hand_the_same_player_two_byes() {
        let es = entrants(3);
        let ps = vec![
            game(1, 1, 2, PairingResult::White),
            PairingRecord { round: 1, white: id(3), black: None, result: Some(PairingResult::Bye) },
        ];
        let pairs = swiss_pairings(&tally(&es, &ps));
        let bye = pairs.iter().find(|(_, b)| b.is_none()).unwrap();
        assert_ne!(bye.0, id(3));
    }

    #[test]
    fn swiss_avoids_rematches_across_a_whole_event() {
        // Six players, five rounds is a full round robin's worth: every
        // round must be rematch-free, which only works if the search
        // backtracks rather than pairing greedily into a corner.
        let es = entrants(6);
        let mut history: Vec<PairingRecord> = Vec::new();
        for round in 1..=5 {
            let pairs = swiss_pairings(&tally(&es, &history));
            assert_everyone_paired_once(&pairs, 6);
            for (w, b) in pairs {
                let b = b.expect("even field has no byes");
                let met = history
                    .iter()
                    .any(|p| (p.white == w && p.black == Some(b)) || (p.white == b && p.black == Some(w)));
                assert!(!met, "round {round} repeated {w} v {b}");
                // Higher seed always wins, so scores spread out realistically.
                let result = if w < b { PairingResult::White } else { PairingResult::Black };
                history.push(PairingRecord { round, white: w, black: Some(b), result: Some(result) });
            }
        }
    }

    #[test]
    fn swiss_allows_a_rematch_only_when_there_is_no_alternative() {
        let es = entrants(2);
        let ps = vec![game(1, 1, 2, PairingResult::White)];
        let pairs = swiss_pairings(&tally(&es, &ps));
        assert_eq!(pairs.len(), 1);
        // And the colours flip for the return game.
        assert_eq!(pairs[0], (id(2), Some(id(1))));
    }

    #[test]
    fn swiss_skips_withdrawn_players() {
        let mut es = entrants(4);
        es[1].withdrawn = true;
        let pairs = swiss_pairings(&tally(&es, &[]));
        assert!(pairs.iter().all(|(w, b)| *w != id(2) && *b != Some(id(2))));
        assert_everyone_paired_once(&pairs, 3);
    }

    #[test]
    fn round_robin_length() {
        assert_eq!(round_robin_rounds(0), 0);
        assert_eq!(round_robin_rounds(1), 0);
        assert_eq!(round_robin_rounds(2), 1);
        assert_eq!(round_robin_rounds(5), 5);
        assert_eq!(round_robin_rounds(6), 5);
    }

    #[test]
    fn round_robin_everyone_meets_everyone_exactly_once() {
        for n in [2usize, 3, 4, 5, 6, 7, 8, 11, 16] {
            let es = entrants(n);
            let mut met: HashSet<(Uuid, Uuid)> = HashSet::new();
            let mut byes: HashMap<Uuid, u32> = HashMap::new();
            for round in 1..=round_robin_rounds(n) {
                let pairs = round_robin_round(&es, round);
                assert_everyone_paired_once(&pairs, n);
                for (w, b) in pairs {
                    match b {
                        Some(b) => {
                            let key = if w < b { (w, b) } else { (b, w) };
                            assert!(met.insert(key), "n={n}: {w} v {b} repeated");
                        }
                        None => *byes.entry(w).or_default() += 1,
                    }
                }
            }
            assert_eq!(met.len(), n * (n - 1) / 2, "n={n}");
            if n % 2 == 1 {
                assert_eq!(byes.len(), n, "n={n}: everyone sits out once");
                assert!(byes.values().all(|&c| c == 1));
            } else {
                assert!(byes.is_empty());
            }
        }
    }

    #[test]
    fn round_robin_colours_stay_balanced() {
        let es = entrants(8);
        let mut whites: HashMap<Uuid, i32> = HashMap::new();
        for round in 1..=7 {
            for (w, _) in round_robin_round(&es, round) {
                *whites.entry(w).or_default() += 1;
            }
        }
        // Seven games each: nobody should be further out than 4–3 / 3–4
        // by more than one game.
        for e in &es {
            let w = whites.get(&e.id).copied().unwrap_or(0);
            assert!((2..=5).contains(&w), "{} had {w} whites of 7", e.id);
        }
    }

    #[test]
    fn round_robin_withdrawal_turns_the_game_into_a_bye() {
        let mut es = entrants(4);
        es[3].withdrawn = true;
        for round in 1..=3 {
            let pairs = round_robin_round(&es, round);
            assert!(pairs.iter().all(|(w, b)| *w != id(4) && *b != Some(id(4))));
            assert_eq!(pairs.iter().filter(|(_, b)| b.is_none()).count(), 1);
        }
    }

    #[test]
    fn arena_pairs_neighbours_and_leaves_the_odd_one_waiting() {
        let t = tally(&entrants(5), &[]);
        let pairs = arena_pairings(&t);
        assert_eq!(pairs.len(), 2);
        let paired: HashSet<Uuid> = pairs.iter().flat_map(|(w, b)| [*w, *b]).collect();
        assert_eq!(paired.len(), 4);
        assert!(!paired.contains(&id(5)));
    }

    #[test]
    fn arena_avoids_an_immediate_rematch_when_it_can() {
        let es = entrants(4);
        let ps = vec![game(0, 1, 2, PairingResult::White), game(0, 3, 4, PairingResult::White)];
        let pairs = arena_pairings(&tally(&es, &ps));
        for (w, b) in &pairs {
            let rematch = ps.iter().any(|p| {
                (p.white == *w && p.black == Some(*b)) || (p.white == *b && p.black == Some(*w))
            });
            assert!(!rematch, "{w} v {b} just played");
        }
        assert_eq!(pairs.len(), 2);
    }

    #[test]
    fn arena_lets_two_players_alone_keep_playing_with_colours_swapped() {
        let es = entrants(2);
        let ps = vec![game(0, 1, 2, PairingResult::White)];
        let pairs = arena_pairings(&tally(&es, &ps));
        assert_eq!(pairs, vec![(id(2), id(1))]);
    }

    #[test]
    fn validation_rejects_what_the_site_cannot_run() {
        let base = NewTournament {
            name: "Club night".into(),
            format: TournamentFormat::Swiss,
            venue: TournamentVenue::Otb,
            initial_seconds: 600,
            increment_seconds: 5,
            rated: false,
            starts_in_minutes: 0,
            duration_minutes: 60,
            rounds: 5,
        };
        assert!(base.validate().is_ok());
        assert!(NewTournament { name: " a ".into(), ..base.clone() }.validate().is_err());
        assert!(NewTournament { format: TournamentFormat::Arena, ..base.clone() }.validate().is_err());
        assert!(NewTournament { rated: true, ..base.clone() }.validate().is_err());
        assert!(NewTournament { rounds: 0, ..base.clone() }.validate().is_err());
        assert!(
            NewTournament {
                format: TournamentFormat::Arena,
                venue: TournamentVenue::Online,
                duration_minutes: 5,
                ..base.clone()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn enum_strings_round_trip() {
        for f in [TournamentFormat::Arena, TournamentFormat::Swiss, TournamentFormat::RoundRobin] {
            assert_eq!(TournamentFormat::parse(f.as_str()), Some(f));
        }
        for v in [TournamentVenue::Online, TournamentVenue::Otb] {
            assert_eq!(TournamentVenue::parse(v.as_str()), Some(v));
        }
        for s in [
            TournamentStatus::Upcoming,
            TournamentStatus::Live,
            TournamentStatus::Finished,
            TournamentStatus::Cancelled,
        ] {
            assert_eq!(TournamentStatus::parse(s.as_str()), Some(s));
        }
        for r in [
            PairingResult::White,
            PairingResult::Black,
            PairingResult::Draw,
            PairingResult::Bye,
            PairingResult::Void,
        ] {
            assert_eq!(PairingResult::parse(r.as_str()), Some(r));
        }
    }
}
