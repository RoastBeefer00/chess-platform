# Features

## Existing Features

### Authentication & Accounts
- OAuth login via GitHub and Google (OpenID Connect)
- Username selection after first login
- User profiles (avatar, bio, country)
- Redis-backed HTTP-only session cookies
- CSRF-protected OAuth flows

### Matchmaking
- Rated and casual (unrated) queues
- Rating-window pairing via Redis Lua script
- Game mode selection: Bullet, Blitz, Rapid, Classical
- Variant selection: Standard, Chess960
- Time control modes: Increment and Bronstein Delay

### Live Chess
- Real-time multiplayer over WebSocket
- Legal move validation via `shakmaty` (server-side)
- Drag-and-drop piece movement
- Legal move dot hints
- Square highlights (last move, selected piece)
- Synchronized clocks (server-authoritative with broadcast sync)
- Clock modes: increment and delay
- In-game chat
- Draw offers and acceptance/decline
- Rematch offers and acceptance/decline
- Resignation
- Timeout detection and flagging
- Full game-over modal with result display

### Sound Effects
- Move, capture, check, checkmate
- Victory, defeat, draw
- Low time warning
- Game start notification
- Select and error feedback

### Ratings & History
- ELO rating system per game mode (5 separate ratings)
- K-factor varies by games played and rating level (32 / 24 / 16)
- Atomic ELO update on game finalization
- Rating history tracked per game

### Infrastructure
- Axum SSR + WASM hydration via Leptos 0.8
- PostgreSQL database with 7 migrations
- Rate limiting (tower_governor): 6 req/s OAuth, 2 req/s API
- Multi-stage Docker build for Fly.io deployment
- CSP, HSTS, X-Frame-Options, Referrer-Policy headers

---

## Potential Future Features

### Gameplay Improvements
- **Move table** — interactive list of moves shown alongside the board (SAN notation, clickable to jump to that position)
- **Analysis board** — post-game board with free move exploration and engine evaluation (Stockfish via WASM)
- **Pre-moves** — queue a move while the opponent is thinking
- **Board flip** — manually flip board orientation mid-game
- **Move confirmation** — optional click-to-confirm before submitting a move (useful on mobile)
- **Takeback requests** — ask opponent to undo the last move
- **Abort / no-show handling** — auto-abort if opponent never connects within N seconds

### Puzzles
- **Daily puzzle** — a new tactical puzzle served each day
- **Puzzle rush** — solve as many puzzles as possible before time runs out
- **Puzzle themes** — filter by motif (fork, pin, skewer, back rank, etc.)
- **Puzzle rating** — separate ELO for puzzle performance

### Game History & Analysis
- **Game archive** — list of past games per user with results and ratings
- **Replay viewer** — step through any completed game move-by-move
- **Engine analysis overlay** — show engine best move / evaluation bar on completed games
- **Opening explorer** — identify opening from move sequence, link to ECO database
- **Accuracy score** — post-game accuracy percentage (based on engine deviation)

### Customization
- **Board themes** — multiple board color schemes (classic, blue, green, etc.)
- **Piece themes** — swap piece sprite sets (Classic, Neo, Alpha, etc.)
- **Board size** — adjustable board size for different screen sizes
- **Sound on/off toggle** — mute/unmute all sound effects in settings
- **Move animation speed** — adjustable or disableable piece slide animation
- **Auto-queen promotion** — skip the promotion picker and always promote to queen

### User Profiles & Social
- **Rich profile page** — show ELO charts per time control, win/loss/draw stats, recent games, total games played
- **Country flag display** — show country alongside username (field exists in DB, not yet surfaced in UI)
- **Avatar upload** — replace OAuth-provided avatar with a custom one
- **Bio editing** — editable bio field in profile settings
- **Friends list** — add/remove friends, see their online status
- **Direct challenge** — send a challenge to a friend with custom time controls
- **Follow / spectate** — watch a friend's live game in real time
- **Leaderboard** — top-rated players per time control

### Spectating & Tournaments
- **Live spectator mode** — watch ongoing games (broadcast channel already supports multi-client)
- **Featured games** — surface high-rated or interesting live games on the home page
- **Tournaments** — round-robin or Swiss brackets with automated pairing
- **Simuls** — one player vs. many simultaneously

### Notifications & Communication
- **In-game chat history** — persist chat messages so they survive page refresh
- **Rematch lobby** — persistent room after game over for casual rematches without re-queuing
- **Browser notifications** — notify when a match is found or a challenge arrives
- **Email notifications** — opt-in for game results or friend activity

### Platform & Infrastructure
- **Move persistence** — write every move to the DB so in-progress games survive server restarts (tracked in `docs/game-saving-elo.md`)
- **Spectator WebSocket route** — hand-rolled axum WS route with `max_message_size` for chat safety
- **Game seek / open challenges** — post an open challenge that any player can accept
- **Mobile-native UI** — touch-optimized drag or tap-to-move controls
- **Keyboard navigation** — arrow keys to step through moves in analysis/replay
- **PWA / installable app** — service worker and web app manifest for mobile install
