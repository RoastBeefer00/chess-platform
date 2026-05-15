CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
