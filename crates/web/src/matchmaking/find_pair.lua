-- KEYS[1]  = bucket sorted set, e.g. "mm:blitz:300+0:rated"
-- ARGV[1]  = requesting user_id (string)
-- ARGV[2]  = requesting user's rating (number, the score they were added with)
-- ARGV[3]  = rating window (number, e.g. 100 means ±100)
--
-- Returns: matched opponent's user_id (string) or false (nil) if no match found.

local bucket   = KEYS[1]
local me       = ARGV[1]
local rating   = tonumber(ARGV[2])
local window   = tonumber(ARGV[3])

-- Confirm requester is still in the queue (could have been paired by another
-- script run between the join and this attempt).
if redis.call('ZSCORE', bucket, me) == false then
return false
end

-- Find candidates within rating window, ordered by rating ascending.
-- LIMIT 0 50 caps the scan; enough to skip past self + pick best.
local candidates = redis.call(
'ZRANGEBYSCORE',
bucket,
rating - window,
rating + window,
'WITHSCORES',
'LIMIT', 0, 50
)

-- candidates is a flat list: { user1, score1, user2, score2, ... }
local best_id    = nil
local best_delta = nil

for i = 1, #candidates, 2 do
local cand_id    = candidates[i]
local cand_score = tonumber(candidates[i + 1])
if cand_id ~= me then
local delta = math.abs(cand_score - rating)
if best_delta == nil or delta < best_delta then
best_id    = cand_id
best_delta = delta
end
end
end

if best_id == nil then
return false
end

-- Atomic pair removal. If either ZREM returns 0, opponent was already paired
-- by a racing script — abort and let caller retry (rare but possible if you
-- spawn matchers in parallel; with a single matcher loop, can't happen).
local removed_me  = redis.call('ZREM', bucket, me)
local removed_opp = redis.call('ZREM', bucket, best_id)

if removed_me == 0 or removed_opp == 0 then
-- Reinsert whichever one was successfully removed to keep state consistent.
if removed_me == 1 then
redis.call('ZADD', bucket, rating, me)
end
if removed_opp == 1 then
-- We don't know opp's exact rating anymore; re-add with their last seen score.
local opp_score = nil
for i = 1, #candidates, 2 do
if candidates[i] == best_id then
  opp_score = tonumber(candidates[i + 1])
  break
end
end
if opp_score then
redis.call('ZADD', bucket, opp_score, best_id)
end
end
return false
end

return best_id
