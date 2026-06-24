use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{PlayerRole, Side};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameOverReason {
    Abort,
    Checkmate,
    Stalemate,
    InsufficientMaterial,
    Repetition,
    FiftyMove,
    DrawAgreement,
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
    /// `think_ms` is the client-measured time between rendering the position
    /// the player moved in and committing this move. The server charges this
    /// (bounded) against the mover's clock, so a premove — committed the
    /// instant the opponent's move renders — reports ~0 and costs ~0,
    /// independent of network latency.
    MoveMade { uci: String, think_ms: i64 },
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
    use crate::{PlayerRole, Side};
    use uuid::Uuid;

    fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
        val: &T,
    ) {
        let json = serde_json::to_string(val).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*val, back);
    }

    // ── rtt_bucket (original) ────────────────────────────────────────────────

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

    // ── GameClientMessage serde ──────────────────────────────────────────────

    #[test]
    fn client_user_joined_roundtrip() {
        roundtrip(&GameClientMessage::UserJoined { game_id: Uuid::new_v4() });
    }

    #[test]
    fn client_move_made_roundtrip() {
        roundtrip(&GameClientMessage::MoveMade { uci: "e2e4".to_string(), think_ms: 0 });
        roundtrip(&GameClientMessage::MoveMade { uci: "g1f3".to_string(), think_ms: 1234 });
    }

    #[test]
    fn client_ping_roundtrip() {
        roundtrip(&GameClientMessage::Ping { client_time_ms: 1_700_000_000_000 });
    }

    #[test]
    fn client_resignation_and_draw_variants_roundtrip() {
        for msg in [
            GameClientMessage::Resign,
            GameClientMessage::DrawOffer,
            GameClientMessage::DrawAccept,
            GameClientMessage::DrawDecline,
            GameClientMessage::RematchOffer,
            GameClientMessage::RematchAccept,
            GameClientMessage::RematchDecline,
            GameClientMessage::RematchCancel,
        ] {
            roundtrip(&msg);
        }
    }

    // ── GameServerMessage serde ──────────────────────────────────────────────

    #[test]
    fn server_user_joined_roundtrip() {
        let msg = GameServerMessage::UserJoined {
            uuid: Uuid::new_v4(),
            position_fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1".to_string(),
            player_role: PlayerRole::Player(Side::White),
            moves: vec!["e2e4".to_string(), "e7e5".to_string()],
            white_wins: 1.5,
            black_wins: 0.5,
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_move_made_roundtrip() {
        let msg = GameServerMessage::MoveMade {
            uci: "e2e4".to_string(),
            white_ms_left: 300_000,
            black_ms_left: 299_800,
            turn: Side::Black,
            sent_at_ms: 1_700_000_000_000,
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_clock_sync_roundtrip() {
        let msg = GameServerMessage::ClockSync {
            white_ms_left: 150_000,
            black_ms_left: 148_000,
            turn: Side::White,
            sent_at_ms: 1_700_000_000_000,
            clock_running: true,
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_resync_roundtrip() {
        let msg = GameServerMessage::Resync {
            position_fen: "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1".to_string(),
            moves: vec!["e2e4".to_string()],
            white_ms_left: 599_900,
            black_ms_left: 600_000,
            turn: Side::Black,
            sent_at_ms: 1_700_000_000_000,
            clock_running: true,
            white_wins: 0.0,
            black_wins: 0.0,
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_game_over_white_wins_roundtrip() {
        let msg = GameServerMessage::GameOver {
            winner: Some(Side::White),
            reason: GameOverReason::Checkmate,
            white_wins: 1.0,
            black_wins: 0.0,
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_game_over_draw_roundtrip() {
        let msg = GameServerMessage::GameOver {
            winner: None,
            reason: GameOverReason::Stalemate,
            white_wins: 0.5,
            black_wins: 0.5,
        };
        roundtrip(&msg);
    }

    /// Both fields Some = countdown active; both None = countdown cleared.
    #[test]
    fn server_abort_countdown_active_roundtrip() {
        let msg = GameServerMessage::AbortCountdown {
            side: Some(Side::White),
            deadline_ms: Some(1_700_000_015_000),
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_abort_countdown_cleared_roundtrip() {
        let msg = GameServerMessage::AbortCountdown {
            side: None,
            deadline_ms: None,
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_presence_update_with_rtt_roundtrip() {
        let msg = GameServerMessage::PresenceUpdate {
            side: Side::Black,
            connected: true,
            rtt_ms: Some(42),
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_presence_update_no_rtt_roundtrip() {
        let msg = GameServerMessage::PresenceUpdate {
            side: Side::White,
            connected: false,
            rtt_ms: None,
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_pong_roundtrip() {
        let msg = GameServerMessage::Pong {
            client_time_ms: 1_700_000_000_000,
            server_time_ms: 1_700_000_000_010,
        };
        roundtrip(&msg);
    }

    #[test]
    fn server_all_game_over_reasons_roundtrip() {
        for reason in [
            GameOverReason::Abort,
            GameOverReason::Checkmate,
            GameOverReason::Stalemate,
            GameOverReason::InsufficientMaterial,
            GameOverReason::Repetition,
            GameOverReason::FiftyMove,
            GameOverReason::DrawAgreement,
            GameOverReason::Timeout,
            GameOverReason::Resignation,
        ] {
            let msg = GameServerMessage::GameOver {
                winner: None,
                reason,
                white_wins: 0.5,
                black_wins: 0.5,
            };
            roundtrip(&msg);
        }
    }
}
