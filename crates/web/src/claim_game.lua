-- KEYS[1]  = active_games:{game_id}
-- ARGV[1]  = white_id
-- ARGV[2]  = black_id
-- ARGV[3]  = category
-- ARGV[4]  = rated ("1"/"0")
-- ARGV[5]  = fen
-- ARGV[6]  = owner_instance (the adopting instance)
-- ARGV[7]  = TTL in seconds
--
-- Atomic compare-and-set for game adoption: claims ownership of a game
-- whose active_games hash is missing (heartbeat lapsed / never existed),
-- but only if it's still missing at the moment this runs. Plain
-- HSET+EXPIRE (as active_game_upsert uses for a *new* game) would let two
-- instances racing to adopt the same orphan both "win" and split-brain the
-- two players onto separate rooms.
--
-- Returns 1 if this call created the record (claim won), 0 if the key
-- already existed (a peer beat us to it, or the record was never
-- actually gone).

local key = KEYS[1]

if redis.call('EXISTS', key) == 1 then
  return 0
end

redis.call('HSET', key,
  'white_id', ARGV[1],
  'black_id', ARGV[2],
  'category', ARGV[3],
  'rated', ARGV[4],
  'fen', ARGV[5],
  'owner_instance', ARGV[6]
)
redis.call('EXPIRE', key, tonumber(ARGV[7]))
return 1
