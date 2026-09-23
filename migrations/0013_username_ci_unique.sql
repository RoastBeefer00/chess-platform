-- Usernames were unique only case-sensitively (the `UNIQUE` on the column
-- added in 0004), so `Magnus`, `magnus` and `MAGNUS` could all coexist and
-- render as three indistinguishable players. Uniqueness is now on the
-- lowercased value, which is also what `is_username_available` and
-- `find_by_username` match on so a profile URL is case-insensitive too.
--
-- If this migration fails with a uniqueness violation, existing rows
-- already collide case-insensitively; resolve those by hand first
--   SELECT lower(username), count(*) FROM users WHERE username IS NOT NULL
--   GROUP BY 1 HAVING count(*) > 1;
-- and re-run. NULL usernames (users who haven't onboarded) are exempt, as
-- they were under the old constraint.
CREATE UNIQUE INDEX IF NOT EXISTS users_username_lower_idx
    ON users (LOWER(username))
    WHERE username IS NOT NULL;
