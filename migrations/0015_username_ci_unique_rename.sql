-- Repairs 0013, which never did anything.
--
-- 0012 creates `users_username_lower_idx` as a plain `text_pattern_ops`
-- index for the friend-search prefix match in `FriendStore::search`. 0013
-- then tried to create the case-insensitive *uniqueness* index under that
-- same name with `IF NOT EXISTS` — and `IF NOT EXISTS` matches on the name,
-- not the definition. So on any database that ran 0012 first (that is: every
-- database), 0013 is a silent no-op that still records itself as applied.
--
-- The two environments drifted apart as a result. Production kept 0012's
-- prefix index and never gained uniqueness, so `set_username`'s
-- unique-violation guard — the thing that actually decides a race between
-- two people claiming `Magnus` and `magnus` — was not being enforced there.
-- Development had the opposite: the uniqueness index was created by hand
-- under the colliding name while untangling a half-applied 0013, which
-- destroyed 0012's prefix index and left friend search doing a seq scan.
--
-- Both indexes are wanted; they only ever collided on the name. This gives
-- uniqueness its own, and puts the prefix index back where it is missing.
-- Written against the indexes actually present rather than against any
-- assumed migration state, so it reaches the same end state from either
-- environment's starting point, and no-ops if run again.

-- A database where `users_username_lower_idx` is the UNIQUE variant is one
-- where the repair was made by hand. Its job moves to the name below, so
-- drop it here and let the next statement restore 0012's index under it.
-- Dropping and recreating within one migration means the uniqueness is
-- never actually absent: sqlx runs each migration in a transaction.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_index i
        JOIN pg_class c ON c.oid = i.indexrelid
        WHERE c.relname = 'users_username_lower_idx'
          AND i.indisunique
    ) THEN
        DROP INDEX users_username_lower_idx;
    END IF;
END $$;

-- 0012's index, for `lower(username) LIKE lower($1) || '%'`.
CREATE INDEX IF NOT EXISTS users_username_lower_idx
    ON users (lower(username) text_pattern_ops);

-- What 0013 meant to create, under a name that doesn't collide. Partial on
-- `username IS NOT NULL` so users who haven't onboarded are exempt, exactly
-- as they were under the case-sensitive `users_username_key` from 0004.
--
-- If this fails with a uniqueness violation, rows already collide
-- case-insensitively; find them with
--   SELECT lower(username), count(*) FROM users WHERE username IS NOT NULL
--   GROUP BY 1 HAVING count(*) > 1;
-- resolve by hand, and re-run.
CREATE UNIQUE INDEX IF NOT EXISTS users_username_lower_unique_idx
    ON users (LOWER(username))
    WHERE username IS NOT NULL;
