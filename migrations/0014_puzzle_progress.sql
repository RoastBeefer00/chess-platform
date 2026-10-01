-- Puzzle progress. Solving a puzzle previously left no trace at all: no
-- history, no rating, nothing to come back for.
--
-- Puzzle rating rides on the existing `ratings`/`rating_history` tables
-- under a new 'puzzle' mode rather than getting its own parallel pair, so
-- it reuses `elo::new_rating`, the per-user seeding trigger, and the
-- rating-history query the profile already draws from. The opponent in that
-- Elo calculation is the puzzle itself, at its own stored rating.

ALTER TABLE ratings DROP CONSTRAINT ratings_mode_check;
ALTER TABLE ratings ADD CONSTRAINT ratings_mode_check
    CHECK (mode IN ('bullet','blitz','rapid','classical','960','puzzle'));

ALTER TABLE rating_history DROP CONSTRAINT rating_history_mode_check;
ALTER TABLE rating_history ADD CONSTRAINT rating_history_mode_check
    CHECK (mode IN ('bullet','blitz','rapid','classical','960','puzzle'));

-- Same trigger as 0006, with 'puzzle' added.
CREATE OR REPLACE FUNCTION seed_user_ratings() RETURNS trigger AS $$
BEGIN
    INSERT INTO ratings (user_id, mode) VALUES
        (NEW.id, 'bullet'),
        (NEW.id, 'blitz'),
        (NEW.id, 'rapid'),
        (NEW.id, 'classical'),
        (NEW.id, '960'),
        (NEW.id, 'puzzle');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Backfill the new mode for users who already exist.
INSERT INTO ratings (user_id, mode)
SELECT u.id, 'puzzle' FROM users u
ON CONFLICT DO NOTHING;

-- One row per (user, puzzle). The primary key is what makes a re-attempt
-- idempotent: a puzzle only ever moves someone's rating the first time they
-- see it, so grinding the same puzzle can't farm rating.
CREATE TABLE puzzle_attempts (
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    puzzle_id    TEXT NOT NULL REFERENCES puzzles(id) ON DELETE CASCADE,
    solved       BOOLEAN NOT NULL,
    -- The solver's puzzle rating before and after this attempt, so a
    -- history view can show the delta without recomputing it.
    rating_before INT NOT NULL,
    rating_after  INT NOT NULL,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, puzzle_id)
);

CREATE INDEX puzzle_attempts_user_time_idx
    ON puzzle_attempts (user_id, attempted_at DESC);
