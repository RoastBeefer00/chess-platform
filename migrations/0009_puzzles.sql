CREATE TABLE puzzles (
    id               TEXT PRIMARY KEY,      -- lichess's own id, e.g. "00sHx"
    fen              TEXT NOT NULL,
    moves            TEXT NOT NULL,          -- space-separated UCI, lichess's own convention
    rating           INT NOT NULL,
    rating_deviation INT NOT NULL,
    popularity       INT NOT NULL,
    nb_plays         INT NOT NULL,
    themes           TEXT NOT NULL,          -- space-separated tags, matches the vendored icon filenames
    game_url         TEXT,
    opening_tags     TEXT
);
CREATE INDEX puzzles_rating_idx ON puzzles (rating);

-- Ingest a CSV slice of lichess's own puzzle database
-- (database.lichess.org/#puzzles, CC0) with columns in this exact order:
-- PuzzleId, FEN, Moves, Rating, RatingDeviation, Popularity, NbPlays,
-- Themes, GameUrl, OpeningTags — matches this table's column order, so
-- COPY needs no transformation:
--   psql "$DATABASE_URL" -c "\copy puzzles FROM 'path/to/slice.csv' WITH (FORMAT csv, HEADER true)"
