-- KEYS[1]  = active_games:{game_id}
-- ARGV[1]  = fen
-- ARGV[2]  = white_ms_left
-- ARGV[3]  = black_ms_left
-- ARGV[4]  = clock_sent_at_ms (server UNIX epoch ms the clocks are true as of)
-- ARGV[5]  = TTL in seconds
--
-- Per-move partial update: refresh the position and clocks and, with them,
-- the ownership TTL (active play is itself a heartbeat signal).
--
-- The clocks are mirrored here so a watch grid running on a DIFFERENT
-- instance can render this game with live clocks. It has no `GameRoom` to
-- read them from, only this hash. They are stored as a snapshot plus the
-- epoch ms it was taken at, rather than a remaining-time figure that would
-- start rotting the moment it was written — the client already knows how to
-- tick a (snapshot, taken_at) pair down locally, which is exactly what it
-- does for a local game.
--
-- Only touches a record that still exists. A bare HSET would happily
-- recreate a key that had just expired — but with only these fields, no
-- owner_instance and no TTL, which is worse than the expiry it papered
-- over: the routing middleware reads no owner, the watch grid skips the
-- malformed entry, and nothing ever expires it. Leaving it to
-- heartbeat_game.lua's recreate path instead means the record comes back
-- complete, within one heartbeat interval.
--
-- Returns 1 on update, 0 if the record was gone.

local key = KEYS[1]

if redis.call('EXISTS', key) == 0 then
  return 0
end

redis.call('HSET', key,
  'fen', ARGV[1],
  'white_ms', ARGV[2],
  'black_ms', ARGV[3],
  'clock_at', ARGV[4]
)
redis.call('EXPIRE', key, tonumber(ARGV[5]))
return 1
