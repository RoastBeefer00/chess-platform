-- Tournaments: scheduled daily arenas, plus user-organized events that are
-- either played on the site ('online') or over the board ('otb', where the
-- site only keeps pairings and scores).

CREATE TABLE tournaments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,

    format TEXT NOT NULL CHECK (format IN ('arena','swiss','round_robin')),
    venue  TEXT NOT NULL CHECK (venue IN ('online','otb')),
    status TEXT NOT NULL DEFAULT 'upcoming'
        CHECK (status IN ('upcoming','live','finished','cancelled')),

    -- NULL for the site's own scheduled events.
    organizer_id UUID REFERENCES users(id) ON DELETE SET NULL,
    -- Which recurring schedule produced this row ('daily-blitz', ...). NULL
    -- for anything a user created. Paired with `starts_at` below it is the
    -- idempotency key that lets every instance lazily "ensure today's
    -- tournaments exist" without coordinating.
    schedule_key TEXT,

    time_initial_seconds   INT NOT NULL,
    time_increment_seconds INT NOT NULL DEFAULT 0,
    rated BOOLEAN NOT NULL DEFAULT FALSE,

    starts_at TIMESTAMPTZ NOT NULL,
    -- Arena only: the event runs for a fixed window and pairs continuously.
    duration_minutes INT,
    -- Swiss / round robin only. Round robin fills this in when it starts,
    -- once the field is known.
    total_rounds  INT,
    current_round INT NOT NULL DEFAULT 0,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at   TIMESTAMPTZ,

    -- An arena is continuous pairing against a clock; there is nothing for
    -- an over-the-board organizer to run.
    CHECK (format <> 'arena' OR venue = 'online'),
    CHECK (format <> 'arena' OR duration_minutes IS NOT NULL),
    -- Rated results come from games the server adjudicated itself.
    CHECK (venue = 'online' OR rated = FALSE)
);

CREATE UNIQUE INDEX tournaments_schedule_slot_idx
    ON tournaments (schedule_key, starts_at) WHERE schedule_key IS NOT NULL;
CREATE INDEX tournaments_open_idx
    ON tournaments (starts_at) WHERE status IN ('upcoming','live');
CREATE INDEX tournaments_organizer_idx ON tournaments (organizer_id);

CREATE TABLE tournament_players (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tournament_id UUID NOT NULL REFERENCES tournaments(id) ON DELETE CASCADE,
    -- NULL for an over-the-board entrant with no account on the site.
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    display_name TEXT,
    -- Join order. Fixes the round-robin schedule and breaks standings ties.
    seed INT NOT NULL,
    withdrawn BOOLEAN NOT NULL DEFAULT FALSE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CHECK (user_id IS NOT NULL OR display_name IS NOT NULL),
    -- NULL user_ids are distinct, so any number of walk-ins can coexist.
    UNIQUE (tournament_id, user_id),
    UNIQUE (tournament_id, seed)
);

CREATE INDEX tournament_players_user_idx ON tournament_players (user_id);

CREATE TABLE tournament_pairings (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tournament_id UUID NOT NULL REFERENCES tournaments(id) ON DELETE CASCADE,
    -- 1-based for swiss / round robin; always 0 for an arena.
    round INT NOT NULL,
    white_player_id UUID NOT NULL REFERENCES tournament_players(id) ON DELETE CASCADE,
    -- NULL = bye.
    black_player_id UUID REFERENCES tournament_players(id) ON DELETE CASCADE,
    -- The game played on the site, when there is one.
    game_id UUID REFERENCES games(id) ON DELETE SET NULL,
    -- NULL = not decided yet. 'void' = the game was aborted; nobody scores.
    result TEXT CHECK (result IN ('white','black','draw','bye','void')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX tournament_pairings_round_idx ON tournament_pairings (tournament_id, round);
-- `finalize_game` settles a pairing by game id on every game that ends, so
-- this lookup has to be an index hit (and a miss, for an ordinary game).
CREATE UNIQUE INDEX tournament_pairings_game_idx
    ON tournament_pairings (game_id) WHERE game_id IS NOT NULL;
