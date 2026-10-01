-- Atomic first-tab/last-tab presence transition, shared across instances.
--
-- The per-user session store is a sorted set scored by the moment each tab
-- last proved it was alive, not a plain set. A plain set only ever shrank
-- when the connection closed cleanly, so any process death — a crash, a
-- SIGKILL, a Fly machine stop, a deploy, a `cargo leptos watch` rebuild —
-- orphaned every session it was holding, and because nothing expired, those
-- users stayed in the online set forever. Scoring by last-seen means a tab
-- whose process died simply stops being refreshed and ages out, the same
-- way `active_games:{id}` ages out in `heartbeat_game.lua`.
--
-- KEYS[1] = online set,            e.g. "friends:online"
-- KEYS[2] = per-user session zset, e.g. "friends:sess:{user_id}"
-- KEYS[3] = legacy session set,    e.g. "friends:sessions:{user_id}"
-- ARGV[1] = "add" | "touch" | "remove"
-- ARGV[2] = session key, e.g. "{instance_id}:{session_id}"
-- ARGV[3] = user_id (string), added to/removed from KEYS[1]
-- ARGV[4] = now, epoch seconds
-- ARGV[5] = ttl seconds; a session not refreshed within this is presumed dead
--
-- Returns 1 when this call changed the user's online state (their first live
-- tab appeared, or their last one went away), 0 otherwise — the caller
-- broadcasts a PresenceUpdate exactly when it sees 1.

local online   = KEYS[1]
local sessions = KEYS[2]
local legacy   = KEYS[3]
local action   = ARGV[1]
local session  = ARGV[2]
local user_id  = ARGV[3]
local now      = tonumber(ARGV[4])
local ttl      = tonumber(ARGV[5])

-- Any session that stopped heartbeating is gone, whatever we were called to
-- do. This is what makes the structure self-healing rather than dependent on
-- a clean disconnect.
redis.call('ZREMRANGEBYSCORE', sessions, '-inf', now - ttl)

-- The old plain-set key for this user, left behind by a previous version.
-- Dropped on reconnect so the stale keys drain as people come back.
redis.call('DEL', legacy)

if action == 'add' then
    local was_empty = redis.call('ZCARD', sessions) == 0
    redis.call('ZADD', sessions, now, session)
    -- Backstop: if every instance holding this user dies at once and nobody
    -- ever reads them again, the key still disappears on its own.
    redis.call('EXPIRE', sessions, ttl * 2)
    if was_empty then
        redis.call('SADD', online, user_id)
        return 1
    end
    return 0

elseif action == 'touch' then
    -- Only refresh a session that is still registered. A heartbeat arriving
    -- after `remove` must not resurrect it.
    if redis.call('ZSCORE', sessions, session) then
        redis.call('ZADD', sessions, now, session)
        redis.call('EXPIRE', sessions, ttl * 2)
        -- Re-assert membership: a reader may have pruned this user while a
        -- heartbeat was briefly late, and the connection is plainly alive.
        -- SADD returning 1 means we just brought them back, which the caller
        -- announces like any other transition.
        return redis.call('SADD', online, user_id)
    end
    return 0

else
    redis.call('ZREM', sessions, session)
    if redis.call('ZCARD', sessions) == 0 then
        redis.call('SREM', online, user_id)
        redis.call('DEL', sessions)
        return 1
    end
    redis.call('EXPIRE', sessions, ttl * 2)
    return 0
end
