CREATE TABLE games (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    mode TEXT NOT NULL CHECK (mode IN ('bullet','blitz','rapid','classical','960')),

    time_initial_seconds   INT NOT NULL,
    time_increment_seconds INT NOT NULL DEFAULT 0,
    time_delay_seconds     INT NOT NULL DEFAULT 0,
    delay_type             TEXT CHECK (delay_type IN ('bronstein','simple')),

    rated BOOLEAN NOT NULL DEFAULT TRUE,

    white_user_id UUID NOT NULL REFERENCES users(id),
    black_user_id UUID NOT NULL REFERENCES users(id),

    status TEXT NOT NULL CHECK (status IN ('waiting','active','finished','aborted')),
    result TEXT CHECK (result IN ('white','black','draw')),
    termination TEXT CHECK (termination IN (
        'checkmate','resignation','timeout',
        'draw_agreement','stalemate','insufficient_material',
        'fifty_move','repetition','abandonment'
    )),

    initial_fen TEXT,
    moves       TEXT,
    final_fen   TEXT,

    white_rating_before INT,
    black_rating_before INT,
    white_rating_after  INT,
    black_rating_after  INT,

    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at   TIMESTAMPTZ
);

CREATE INDEX games_white_user_idx ON games (white_user_id, ended_at DESC);
CREATE INDEX games_black_user_idx ON games (black_user_id, ended_at DESC);
CREATE INDEX games_active_idx ON games (status) WHERE status = 'active';

CREATE TABLE matchmaking_queue (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    mode TEXT NOT NULL CHECK (mode IN ('bullet','blitz','rapid','classical','960')),
    time_initial_seconds   INT NOT NULL,
    time_increment_seconds INT NOT NULL DEFAULT 0,
    time_delay_seconds     INT NOT NULL DEFAULT 0,
    delay_type             TEXT CHECK (delay_type IN ('bronstein','simple')),
    rated   BOOLEAN NOT NULL DEFAULT TRUE,
    rating  INT NOT NULL,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX matchmaking_queue_search_idx
    ON matchmaking_queue (mode, time_initial_seconds, time_increment_seconds, rating);
