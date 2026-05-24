#[cfg(feature = "hydrate")]
pub fn play(src: &str) {
    if let Ok(audio) = web_sys::HtmlAudioElement::new_with_src(src) {
        let _ = audio.play();
    }
}

#[cfg(not(feature = "hydrate"))]
pub fn play(_src: &str) {}

pub mod sfx {
    pub const MOVE: &str = "/sound/standard/Move.mp3";
    pub const CAPTURE: &str = "/sound/standard/Capture.mp3";
    pub const CHECK: &str = "/sound/standard/Check.mp3";
    pub const CHECKMATE: &str = "/sound/standard/Checkmate.mp3";
    pub const VICTORY: &str = "/sound/standard/Victory.mp3";
    pub const DEFEAT: &str = "/sound/standard/Defeat.mp3";
    pub const DRAW: &str = "/sound/standard/Draw.mp3";
    pub const LOW_TIME: &str = "/sound/standard/LowTime.mp3";
    pub const ERROR: &str = "/sound/standard/Error.mp3";
    pub const SELECT: &str = "/sound/standard/Select.mp3";
    pub const GAME_START: &str = "/sound/standard/GenericNotify.mp3";
}
