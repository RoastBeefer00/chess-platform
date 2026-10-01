-- Which of these users are genuinely online, correcting the online set as it
-- reads.
--
-- `friends:online` is derived state: a user belongs in it exactly while they
-- hold at least one session that is still heartbeating. Writers keep it
-- honest, but a user whose last tab died with its process has no writer left
-- to clean up after it. So the read path prunes too — it ages out that
-- user's dead sessions and drops them from the online set when nothing
-- survives. Presence therefore self-corrects on the next read even if no
-- instance ever touches that user again.
--
-- KEYS[1]   = online set, e.g. "friends:online"
-- KEYS[2..] = one session zset per candidate, e.g. "friends:sess:{user_id}"
-- ARGV[1]   = now, epoch seconds
-- ARGV[2]   = ttl seconds, matching presence.lua
-- ARGV[3..] = candidate user_ids, in the same order as KEYS[2..]
--
-- Returns the subset of candidates that are really online.

local online = KEYS[1]
local now    = tonumber(ARGV[1])
local ttl    = tonumber(ARGV[2])

local result = {}

for i = 2, #KEYS do
    local sessions = KEYS[i]
    local user_id  = ARGV[i + 1]

    if redis.call('SISMEMBER', online, user_id) == 1 then
        redis.call('ZREMRANGEBYSCORE', sessions, '-inf', now - ttl)
        if redis.call('ZCARD', sessions) == 0 then
            redis.call('SREM', online, user_id)
            redis.call('DEL', sessions)
        else
            table.insert(result, user_id)
        end
    end
end

return result
