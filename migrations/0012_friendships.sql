-- One row per *pair*, not per direction. requester_id/addressee_id record
-- who asked whom (needed to render accept/decline vs. cancel); the unique
-- expression index below is what makes the relationship mutual — it rejects
-- both a duplicate A->B and a reverse B->A insert while one already exists,
-- which `FriendStore::send_request` turns into an auto-accept.
--
-- Only two statuses. Declining, cancelling, and unfriending are all DELETEs:
-- a 'declined' row would only exist to be filtered out of every query and to
-- block a later re-request. Revisit with a 'blocked' status if that becomes
-- a problem.
CREATE TABLE friendships (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    requester_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    addressee_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status       TEXT NOT NULL CHECK (status IN ('pending','accepted')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT friendships_no_self CHECK (requester_id <> addressee_id)
);

-- Direction-agnostic uniqueness: (A,B) and (B,A) normalize to the same key,
-- so a pending A->B makes a B->A insert conflict — the signal `send_request`
-- uses to auto-accept instead of erroring.
CREATE UNIQUE INDEX friendships_pair_uniq
    ON friendships (LEAST(requester_id, addressee_id), GREATEST(requester_id, addressee_id));

-- The pair index can't serve `requester_id = $1` alone, so both sides of the
-- accepted set get their own partial index; Postgres BitmapOrs them for the
-- `(requester_id = $1 OR addressee_id = $1)` list query.
CREATE INDEX friendships_accepted_requester_idx ON friendships (requester_id) WHERE status = 'accepted';
CREATE INDEX friendships_accepted_addressee_idx ON friendships (addressee_id) WHERE status = 'accepted';
CREATE INDEX friendships_pending_addressee_idx  ON friendships (addressee_id) WHERE status = 'pending';
CREATE INDEX friendships_pending_requester_idx  ON friendships (requester_id) WHERE status = 'pending';

-- Prefix search for "add a friend". text_pattern_ops keeps
-- `lower(username) LIKE 'foo%'` index-scannable regardless of the database
-- collation. Deliberately not ILIKE '%q%' (infix match can't use any btree
-- index without pg_trgm).
CREATE INDEX users_username_lower_idx ON users (lower(username) text_pattern_ops);
