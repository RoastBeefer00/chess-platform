#[cfg(feature = "hydrate")]
pub fn play(src: &str) {
    if let Ok(audio) = web_sys::HtmlAudioElement::new_with_src(src) {
        let _ = audio.play();
    }
}

#[cfg(not(feature = "hydrate"))]
pub fn play(_src: &str) {}

/// Pick the right sound clip for a move that has just been applied.
/// `pos_after` is the resulting position; `m` is the move played.
/// Branch order matters: checkmate > check > capture > plain move.
pub fn for_move(pos_after: &shakmaty::Chess, m: &shakmaty::Move) -> &'static str {
    use shakmaty::Position as _;
    if matches!(pos_after.outcome(), shakmaty::Outcome::Known(_)) {
        sfx::CHECKMATE
    } else if pos_after.is_check() {
        sfx::CHECK
    } else if m.is_capture() {
        sfx::CAPTURE
    } else {
        sfx::MOVE
    }
}

pub mod sfx {
    pub const MOVE: &str = "/sound/standard/Move.mp3";
    pub const CAPTURE: &str = "/sound/standard/Capture.mp3";
    // The standard/ variants of these two assets ship at ~1.6K — effectively
    // silent. Use the piano/ pack (~17K) so checks and mate are audible.
    pub const CHECK: &str = "/sound/sfx/Check.mp3";
    pub const CHECKMATE: &str = "/sound/sfx/Checkmate.mp3";
    pub const VICTORY: &str = "/sound/standard/Victory.mp3";
    pub const DEFEAT: &str = "/sound/standard/Defeat.mp3";
    pub const DRAW: &str = "/sound/standard/Draw.mp3";
    pub const LOW_TIME: &str = "/sound/standard/LowTime.mp3";
    pub const ERROR: &str = "/sound/standard/Error.mp3";
    pub const SELECT: &str = "/sound/standard/Select.mp3";
    pub const GAME_START: &str = "/sound/standard/GenericNotify.mp3";
}
