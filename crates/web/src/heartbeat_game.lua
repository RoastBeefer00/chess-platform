-- KEYS[1]  = active_games:{game_id}
-- ARGV[1]  = white_id
-- ARGV[2]  = black_id
-- ARGV[3]  = category
-- ARGV[4]  = rated ("1"/"0")
-- ARGV[5]  = fen (used only if the key needs recreating — see below)
-- ARGV[6]  = owner_instance (this instance)
-- ARGV[7]  = white_ms_left
-- ARGV[8]  = black_ms_left
-- ARGV[9]  = clock_sent_at_ms (server UNIX epoch ms the clocks are true as of)
-- ARGV[10] = TTL in seconds
--
-- Self-healing heartbeat for a live game's ownership record. A plain
-- EXPIRE (the previous implementation) can only extend a key that's still
-- there — if it's ever lost for any reason (a transient Redis-side blip,
-- an eviction, anything), EXPIRE on a missing key is a silent no-op, and
-- nothing ever brings it back: the owning room stays alive and healthy in
-- this instance's memory forever, while the reaper perpetually treats the
-- game as ownerless and "re-adopts" it every cycle without ever resolving
-- anything (confirmed live in production on 2026-09-18 — a real game sat
-- in exactly this loop for 20+ minutes straight). This script recreates
-- the record from this room's own known fields when it finds the key
-- gone, so a lost key self-heals within one heartbeat interval instead of
-- staying lost until the process itself restarts.
--
-- Returns 1 if the key already existed (ordinary refresh), 2 if it had to
-- be recreated.

local key = KEYS[1]

if redis.call('EXISTS', key) == 1 then
  redis.call('EXPIRE', key, tonumber(ARGV[10]))
  return 1
end

redis.call('HSET', key,
  'white_id', ARGV[1],
  'black_id', ARGV[2],
  'category', ARGV[3],
  'rated', ARGV[4],
  'fen', ARGV[5],
  'owner_instance', ARGV[6],
  'white_ms', ARGV[7],
  'black_ms', ARGV[8],
  'clock_at', ARGV[9]
)
redis.call('EXPIRE', key, tonumber(ARGV[10]))
return 2
