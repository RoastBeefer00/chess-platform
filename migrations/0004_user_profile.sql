ALTER TABLE users
    ADD COLUMN username TEXT UNIQUE,
    ADD COLUMN avatar_url TEXT,
    ADD COLUMN bio TEXT,
    ADD COLUMN country CHAR(2);
