CREATE TABLE ratings (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    mode    TEXT NOT NULL CHECK (mode IN ('bullet','blitz','rapid','classical','960')),
    rating  INT NOT NULL DEFAULT 1500,
    games   INT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, mode)
);

CREATE TABLE rating_history (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    mode    TEXT NOT NULL CHECK (mode IN ('bullet','blitz','rapid','classical','960')),
    rating  INT NOT NULL,
    game_id UUID REFERENCES games(id) ON DELETE SET NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX rating_history_user_mode_time_idx
    ON rating_history (user_id, mode, recorded_at DESC);

-- Auto-seed a 1500 row for each mode whenever a new user is created.
CREATE OR REPLACE FUNCTION seed_user_ratings() RETURNS trigger AS $$
BEGIN
    INSERT INTO ratings (user_id, mode) VALUES
        (NEW.id, 'bullet'),
        (NEW.id, 'blitz'),
        (NEW.id, 'rapid'),
        (NEW.id, 'classical'),
        (NEW.id, '960');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_seed_ratings
    AFTER INSERT ON users
    FOR EACH ROW
    EXECUTE FUNCTION seed_user_ratings();

-- Backfill rows for users that already exist.
INSERT INTO ratings (user_id, mode)
SELECT u.id, m.mode
FROM users u
CROSS JOIN (VALUES ('bullet'),('blitz'),('rapid'),('classical'),('960')) AS m(mode)
ON CONFLICT DO NOTHING;
