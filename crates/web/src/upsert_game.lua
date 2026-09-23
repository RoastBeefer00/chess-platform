-- KEYS[1]  = active_games:{game_id}
-- ARGV[1]  = white_id
-- ARGV[2]  = black_id
-- ARGV[3]  = category
-- ARGV[4]  = rated ("1"/"0")
-- ARGV[5]  = fen
-- ARGV[6]  = owner_instance
-- ARGV[7]  = white_ms_left
-- ARGV[8]  = black_ms_left
-- ARGV[9]  = clock_sent_at_ms (server UNIX epoch ms the clocks are true as of)
-- ARGV[10] = TTL in seconds
--
-- Unconditional write of a game's ownership/roster record, for a genuinely
-- new game (the adoption path uses claim_game.lua's compare-and-set
-- instead). Exists as a script purely so the HSET and the EXPIRE are one
-- atomic step: as two round trips, a failure or a crash between them
-- leaves a TTL-less active_games key that no heartbeat lapse can ever
-- expire, so the reaper would treat the game as owned by a healthy
-- instance forever.

local key = KEYS[1]

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
return 1
