-- Atomic first-tab/last-tab presence transition, shared across instances.
--
-- KEYS[1] = online set, e.g. "friends:online"
-- KEYS[2] = per-user session set, e.g. "friends:sessions:{user_id}"
-- ARGV[1] = "add" or "remove"
-- ARGV[2] = session key, e.g. "{instance_id}:{session_id}"
-- ARGV[3] = user_id (string), added to/removed from KEYS[1]
--
-- Returns: 1 if this was the user's first tab (add) or last tab (remove),
-- 0 otherwise. The session set's own membership is the tab count — no
-- separate counter needed, mirroring the in-process HashMap it replaces.

local online   = KEYS[1]
local sessions = KEYS[2]
local action   = ARGV[1]
local session  = ARGV[2]
local user_id  = ARGV[3]

if action == 'add' then
local was_empty = redis.call('SCARD', sessions) == 0
redis.call('SADD', sessions, session)
if was_empty then
redis.call('SADD', online, user_id)
return 1
end
return 0
else
redis.call('SREM', sessions, session)
if redis.call('SCARD', sessions) == 0 then
redis.call('SREM', online, user_id)
redis.call('DEL', sessions)
return 1
end
return 0
end
