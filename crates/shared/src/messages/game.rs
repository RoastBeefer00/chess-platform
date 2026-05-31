use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{PlayerRole, Side};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameOverReason {
    Abort,
    Checkmate,
    Draw,
    Timeout,
    Resignation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GameServerMessage {
    UserJoined {
        uuid: Uuid,
        position_fen: String,
        player_role: PlayerRole,
        moves: Vec<String>,
        white_wins: f32,
        black_wins: f32,
    },
    UserLeft {
        username: String,
    },
    MoveMade {
        uci: String,
        white_ms_left: i64,
        black_ms_left: i64,
        turn: Side,
        sent_at_ms: i64,
    },
    Chat {
        user: Uuid,
        text: String,
    },
    ClockSync {
        white_ms_left: i64,
        black_ms_left: i64,
        turn: Side,
        sent_at_ms: i64,
        clock_running: bool,
    },
    /// Authoritative snapshot. Client replaces local position/history/clocks
    /// with these values. Sent when the server rejects a client move or
    /// detects the client has fallen behind (broadcast lag), so the client
    /// can recover without a page refresh.
    Resync {
        position_fen: String,
        moves: Vec<String>,
        white_ms_left: i64,
        black_ms_left: i64,
        turn: Side,
        sent_at_ms: i64,
        clock_running: bool,
        white_wins: f32,
        black_wins: f32,
    },
    GameOver {
        winner: Option<Side>,
        reason: GameOverReason,
        white_wins: f32,
        black_wins: f32,
    },
    RematchOffer {
        from: Uuid,
    },
    RematchAccept {
        new_game_id: Uuid,
    },
    RematchDecline,
    RematchCancel,
    DrawOffer {
        from: Uuid,
    },
    DrawDecline,
    /// First-move abort countdown. Both fields `Some` ⇒ countdown active for
    /// that side until `deadline_ms` (server-time, ms since UNIX epoch). Both
    /// `None` ⇒ countdown cleared (side moved, game ended, etc.).
    AbortCountdown {
        side: Option<Side>,
        deadline_ms: Option<i64>,
    },
    /// Reply to a client `Ping`. Echoes the client's send timestamp and
    /// carries the server's wall-clock time so the client can estimate the
    /// client↔server clock offset (NTP-style) for accurate clock display.
    Pong {
        client_time_ms: i64,
        server_time_ms: i64,
    },
    /// Per-player connection status. Broadcast to the room when a player
    /// connects, disconnects, or their RTT bucket changes.
    PresenceUpdate {
        side: Side,
        connected: bool,
        rtt_ms: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameClientMessage {
    UserJoined { game_id: Uuid },
    MoveMade { uci: String, client_time_ms: i64 },
    Chat { text: String },
    Resign,
    DrawOffer,
    DrawAccept,
    DrawDecline,
    RematchOffer,
    RematchAccept,
    RematchDecline,
    RematchCancel,
    /// Clock-offset probe. `client_time_ms` is the client's `Date.now()` at
    /// send; the server echoes it in `Pong` so the client can compute RTT.
    Ping { client_time_ms: i64 },
}

/// Bucket a raw RTT (ms) into a 0–3 level for UI display.
/// 0 = excellent (<100 ms), 1 = good (100–249 ms), 2 = fair (250–499 ms), 3 = poor (≥500 ms).
pub fn rtt_bucket(rtt_ms: u32) -> u8 {
    match rtt_ms {
        0..=99 => 0,
        100..=249 => 1,
        250..=499 => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtt_bucket_boundaries() {
        assert_eq!(rtt_bucket(0), 0);
        assert_eq!(rtt_bucket(99), 0);
        assert_eq!(rtt_bucket(100), 1);
        assert_eq!(rtt_bucket(249), 1);
        assert_eq!(rtt_bucket(250), 2);
        assert_eq!(rtt_bucket(499), 2);
        assert_eq!(rtt_bucket(500), 3);
        assert_eq!(rtt_bucket(10_000), 3);
    }
}
