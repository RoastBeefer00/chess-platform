//! Client-side Stockfish, via the vendored Web Worker build under
//! `public/engine/` (see that directory's README for provenance/license).
//! Runs entirely in the browser — no server involvement, matching the
//! analysis board's "no network" design.
//!
//! Split into UCI line parsing (pure, works on every target — unit tested
//! under plain `cargo test`) and the actual `Worker` plumbing (real on
//! `hydrate`, an inert stub everywhere else, so `AnalysisBoard` doesn't need
//! to `#[cfg]`-gate every touch point).

/// A position evaluation, from the side to move's point of view (per the
/// UCI `score` spec) — converting to White's POV for display is the
/// caller's job, since that depends on whose turn it is in the position
/// being shown, not on anything the engine knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Score {
    Cp(i32),
    Mate(i32),
}

#[derive(Debug, Clone)]
pub struct EngineInfo {
    pub depth: u32,
    pub score: Score,
    /// Which of the engine's concurrently-searched candidate lines this is,
    /// 1-indexed per UCI's `multipv` convention (1 = the current best line).
    /// Absent on engines/lines that don't report it, which per spec means
    /// there's only the one line — defaults to 1.
    pub multipv: u32,
    /// Principal variation, as UCI moves from the analyzed position.
    pub pv: Vec<String>,
}

/// Parses one `info ... pv ...` line. Returns `None` for every other kind
/// of engine output (`id`, `option`, `uciok`, `readyok`, bare `info string`,
/// ...) and for aspiration-window `info` lines qualified `upperbound`/
/// `lowerbound` — those aren't a settled evaluation, so it's better to keep
/// showing the previous depth's result than to flash a misleading bound.
///
/// Only ever called from the `hydrate`-only `worker` module below; `allow`
/// rather than `cfg`-gating it so it stays compiled (and unit-testable) on
/// every target regardless.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub fn parse_info_line(line: &str) -> Option<EngineInfo> {
    let mut tokens = line.split_whitespace();
    if tokens.next()? != "info" {
        return None;
    }

    let mut depth = None;
    let mut score = None;
    let mut multipv = None;
    let mut pv = Vec::new();

    let rest: Vec<&str> = tokens.collect();
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "depth" => {
                depth = rest.get(i + 1).and_then(|s| s.parse::<u32>().ok());
                i += 2;
            }
            "multipv" => {
                multipv = rest.get(i + 1).and_then(|s| s.parse::<u32>().ok());
                i += 2;
            }
            "score" => {
                let kind = rest.get(i + 1)?;
                let val: i32 = rest.get(i + 2)?.parse().ok()?;
                let bounded = matches!(rest.get(i + 3), Some(&"upperbound") | Some(&"lowerbound"));
                if !bounded {
                    score = match *kind {
                        "cp" => Some(Score::Cp(val)),
                        "mate" => Some(Score::Mate(val)),
                        _ => None,
                    };
                }
                i += if bounded { 4 } else { 3 };
            }
            "pv" => {
                pv = rest[i + 1..].iter().map(|s| s.to_string()).collect();
                break;
            }
            _ => i += 1,
        }
    }

    if pv.is_empty() {
        return None;
    }
    Some(EngineInfo { depth: depth?, score: score?, multipv: multipv.unwrap_or(1), pv })
}

/// Parses a `bestmove <uci> [ponder <uci>]` line. The move itself isn't
/// used anywhere (the live-updating PV's first move already conveys it);
/// the line's mere presence is what matters — it's UCI's signal that a
/// search has actually ended, used to properly sequence searches (see
/// `worker::EngineHandle::go`) instead of firing overlapping ones.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub fn parse_bestmove_line(line: &str) -> Option<String> {
    line.strip_prefix("bestmove ")?
        .split_whitespace()
        .next()
        .map(str::to_string)
}

/// Replays a principal variation (UCI moves) from `start` into SAN text,
/// stopping at the first move that fails to parse/apply rather than
/// panicking — a PV can be cut short mid-stream if the engine's `info` line
/// arrived while a position change was already in flight.
pub fn pv_to_san(start: &shakmaty::Chess, pv: &[String]) -> Vec<String> {
    use shakmaty::{san::SanPlus, uci::UciMove};
    let mut pos = start.clone();
    let mut out = Vec::with_capacity(pv.len());
    for uci_str in pv {
        let Ok(uci) = uci_str.parse::<UciMove>() else { break };
        let Ok(mv) = uci.to_move(&pos) else { break };
        out.push(SanPlus::from_move_and_play_unchecked(&mut pos, mv).to_string());
    }
    out
}

/// The board squares of `pv`'s first move, for drawing a best-move arrow —
/// `None` if `pv` is empty or its first move doesn't parse/apply against
/// `start` (mirrors `pv_to_san`'s own tolerance for a truncated/bad PV).
pub fn pv_first_move(start: &shakmaty::Chess, pv: &[String]) -> Option<(shakmaty::Square, shakmaty::Square)> {
    let uci: shakmaty::uci::UciMove = pv.first()?.parse().ok()?;
    let mv = uci.to_move(start).ok()?;
    Some((mv.from()?, mv.to()))
}

/// Normalizes a `Score` from the side-to-move's point of view (per the UCI
/// spec) to White's point of view — the one normalization every display in
/// this module needs, done in one place so they can't disagree.
pub fn to_white_pov(score: Score, white_to_move: bool) -> Score {
    match score {
        Score::Cp(cp) => Score::Cp(if white_to_move { cp } else { -cp }),
        Score::Mate(m) => Score::Mate(if white_to_move { m } else { -m }),
    }
}

/// Formats a `Score` already normalized to White's point of view.
pub fn format_white_score(score_white_pov: Score) -> String {
    match score_white_pov {
        Score::Cp(cp) => format!("{:+.1}", f64::from(cp) / 100.0),
        Score::Mate(m) => format!("#{m}"),
    }
}

/// Maps a `Score` (White's point of view) to White's share of the eval
/// bar's fill, in `0.0..=1.0`. A standard logistic centered at 0cp —
/// swings toward the extremes for a decisive advantage but never
/// fully saturates except at an actual forced mate, the same curve
/// lichess/chess.com-style eval bars use so a bar half full still reads as
/// "roughly equal" rather than "someone's about to win."
pub fn white_fraction(score_white_pov: Score) -> f64 {
    match score_white_pov {
        Score::Mate(m) if m > 0 => 1.0,
        Score::Mate(m) if m < 0 => 0.0,
        Score::Mate(_) => 0.5,
        Score::Cp(cp) => 1.0 / (1.0 + (-f64::from(cp) / 400.0).exp()),
    }
}

#[cfg(feature = "hydrate")]
mod worker {
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    use web_sys::{MessageEvent, Worker};

    use super::{parse_bestmove_line, parse_info_line, EngineInfo};

    /// The vendored build is single-threaded (no `SharedArrayBuffer`, no
    /// Cross-Origin-Opener/Embedder-Policy headers needed) with its NNUE net
    /// already compiled in — see `public/engine/README.md`.
    const ENGINE_SCRIPT_URL: &str = "/engine/stockfish-18-lite-single.js";

    /// `stop` is just another queued UCI command — it doesn't synchronously
    /// abort anything. Posting a new `position`/`go` right after it (the
    /// original, naive approach) could still land while the *previous*
    /// search was mid-flight: its trailing `info` lines would then arrive
    /// tagged to a position the UI had already moved past, showing as the
    /// eval randomly flipping sign on every move — and feeding the engine
    /// overlapping `position`/`go` pairs is also a plausible contributor to
    /// the internal `unreachable` / `table index out of bounds` traps this
    /// vendored build hit under ordinary use. `bestmove` is the one UCI
    /// message that reliably marks a search as actually over, so `go` is
    /// tracked through this instead: at most one search in flight, and at
    /// most one pending request (the most recent) queued behind it.
    struct SearchState {
        searching: bool,
        pending_fen: Option<String>,
        // The fen the currently in-flight search is actually running
        // against — distinct from `pending_fen` (queued, not yet started).
        // Threaded back out alongside every `info` line so the caller can
        // convert/replay it against the position that was *actually*
        // searched, not whatever position is on screen by the time the
        // message arrives — those can differ (the board's cursor moves the
        // instant a move is played; the engine's own trailing `info` lines
        // for the position just left keep arriving for a bit after), and
        // conflating them is what caused the eval/PV to flash the wrong
        // sign or wrong line for an instant on every move.
        current_fen: Option<String>,
    }

    /// A running Stockfish Web Worker. Dropping it terminates the worker.
    pub struct EngineHandle {
        worker: Worker,
        state: Rc<RefCell<SearchState>>,
        // Kept alive for the worker's lifetime — dropping this would
        // detach the JS callback while the worker could still call it.
        _onmessage: Closure<dyn FnMut(MessageEvent)>,
        _onerror: Closure<dyn FnMut(web_sys::ErrorEvent)>,
    }

    /// Sends `ucinewgame` then starts a search at `fen`. Unlike a normal
    /// UCI client playing one continuous game, the analysis board jumps
    /// between arbitrary, unrelated positions (rewinding, stepping into
    /// variations) — each search here is unrelated to the last, and
    /// `ucinewgame` tells the engine not to assume continuity with
    /// whatever it just analyzed.
    fn start_search(worker: &Worker, fen: &str, state: &Rc<RefCell<SearchState>>) {
        state.borrow_mut().current_fen = Some(fen.to_string());
        let _ = worker.post_message(&JsValue::from_str("ucinewgame"));
        let _ = worker.post_message(&JsValue::from_str(&format!("position fen {fen}")));
        let _ = worker.post_message(&JsValue::from_str("go movetime 700"));
    }

    impl EngineHandle {
        /// Starts the worker and sends the standard UCI handshake.
        /// `on_info` is called for every principal-variation update the
        /// engine emits (each roughly a deeper/better refinement of the
        /// same search — later calls supersede earlier ones for display),
        /// alongside the fen it was actually computed against — see
        /// `SearchState::current_fen`.
        ///
        /// `on_crash` is called if the vendored engine's own WASM hits an
        /// internal trap and dies. That's a bug inside the vendored binary
        /// itself, not something reachable from here; the caller's job is
        /// to drop this handle and start a fresh one, since the worker
        /// can't recover once its WASM instance has trapped.
        pub fn new(
            on_info: impl Fn(EngineInfo, String) + 'static,
            on_crash: impl Fn() + 'static,
        ) -> Result<Self, String> {
            let worker = Worker::new(ENGINE_SCRIPT_URL).map_err(|e| format!("{e:?}"))?;
            let state = Rc::new(RefCell::new(SearchState {
                searching: false,
                pending_fen: None,
                current_fen: None,
            }));

            let onmessage = {
                let worker = worker.clone();
                let state = state.clone();
                Closure::wrap(Box::new(move |event: MessageEvent| {
                    let Some(line) = event.data().as_string() else { return };
                    if let Some(info) = parse_info_line(&line) {
                        let Some(fen) = state.borrow().current_fen.clone() else { return };
                        on_info(info, fen);
                        return;
                    }
                    if parse_bestmove_line(&line).is_some() {
                        // This search is genuinely over — either start the
                        // most recently requested position (superseding
                        // anything requested in between), or go idle.
                        let mut s = state.borrow_mut();
                        if let Some(fen) = s.pending_fen.take() {
                            drop(s);
                            start_search(&worker, &fen, &state);
                        } else {
                            s.searching = false;
                        }
                    }
                }) as Box<dyn FnMut(MessageEvent)>)
            };
            worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

            let onerror = Closure::wrap(Box::new(move |_event: web_sys::ErrorEvent| {
                on_crash();
            }) as Box<dyn FnMut(web_sys::ErrorEvent)>);
            worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));

            // Stockfish buffers commands it receives before it's fully
            // initialized, so queuing straight through this handshake
            // (rather than waiting for `uciok`/`readyok`) is safe.
            let _ = worker.post_message(&JsValue::from_str("uci"));
            // 3 concurrent candidate lines, for the "top 3 moves" display —
            // set once here rather than per-search since it's a persistent
            // engine option, not part of `position`/`go`.
            let _ = worker.post_message(&JsValue::from_str("setoption name MultiPV value 3"));
            let _ = worker.post_message(&JsValue::from_str("isready"));

            Ok(Self { worker, state, _onmessage: onmessage, _onerror: onerror })
        }

        /// Requests a search at `fen`. If nothing is currently searching,
        /// starts immediately; otherwise sends `stop` (so the in-flight
        /// search wraps up promptly rather than running its full 700ms)
        /// and records `fen` as the position to search once that search's
        /// `bestmove` confirms it's actually finished. Calling this again
        /// before that happens just replaces the pending position — safe
        /// to call on every position change with no external debouncing.
        pub fn go(&self, fen: &str) {
            let mut s = self.state.borrow_mut();
            if s.searching {
                s.pending_fen = Some(fen.to_string());
                let _ = self.worker.post_message(&JsValue::from_str("stop"));
            } else {
                s.searching = true;
                drop(s);
                start_search(&self.worker, fen, &self.state);
            }
        }
    }

    impl Drop for EngineHandle {
        fn drop(&mut self) {
            self.worker.terminate();
        }
    }
}
#[cfg(feature = "hydrate")]
pub use worker::EngineHandle;

/// Inert stand-in for non-hydrate builds (SSR) — never actually
/// constructed, since the "Engine" toggle that would call `new` only does
/// anything client-side. Exists so `AnalysisBoard` can hold an
/// `EngineHandle` unconditionally without `#[cfg]`-splitting its state.
#[cfg(not(feature = "hydrate"))]
pub struct EngineHandle;

#[cfg(not(feature = "hydrate"))]
impl EngineHandle {
    pub fn new(
        _on_info: impl Fn(EngineInfo, String) + 'static,
        _on_crash: impl Fn() + 'static,
    ) -> Result<Self, String> {
        Err("engine unavailable outside the browser".to_string())
    }

    pub fn go(&self, _fen: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cp_score_with_pv() {
        let line = "info depth 14 seldepth 20 multipv 1 score cp 32 nodes 500000 nps 900000 \
                     hashfull 123 tbhits 0 time 250 pv e2e4 e7e5 g1f3 b8c6";
        let info = parse_info_line(line).expect("should parse");
        assert_eq!(info.depth, 14);
        assert_eq!(info.score, Score::Cp(32));
        assert_eq!(info.pv, vec!["e2e4", "e7e5", "g1f3", "b8c6"]);
    }

    #[test]
    fn parses_multipv_index() {
        let line = "info depth 12 multipv 2 score cp -15 pv e7e5 g1f3";
        let info = parse_info_line(line).expect("should parse");
        assert_eq!(info.multipv, 2);
    }

    #[test]
    fn defaults_multipv_to_one_when_absent() {
        let line = "info depth 12 score cp 15 pv e2e4";
        let info = parse_info_line(line).expect("should parse");
        assert_eq!(info.multipv, 1);
    }

    #[test]
    fn parses_mate_score() {
        let line = "info depth 8 score mate 3 pv d1h5 g7g6 h5g6";
        let info = parse_info_line(line).expect("should parse");
        assert_eq!(info.score, Score::Mate(3));
        assert_eq!(info.pv, vec!["d1h5", "g7g6", "h5g6"]);
    }

    #[test]
    fn ignores_bounded_scores() {
        let line = "info depth 10 score cp 900 upperbound pv e2e4";
        assert!(parse_info_line(line).is_none());
    }

    #[test]
    fn ignores_non_info_and_infoless_pv_lines() {
        assert!(parse_info_line("uciok").is_none());
        assert!(parse_info_line("readyok").is_none());
        assert!(parse_info_line("id name Stockfish 18").is_none());
        // No `pv` token at all — e.g. a bare currmove/status line.
        assert!(parse_info_line("info depth 5 currmove e2e4 currmovenumber 1").is_none());
    }

    #[test]
    fn parses_bestmove() {
        assert_eq!(parse_bestmove_line("bestmove e2e4 ponder e7e5"), Some("e2e4".to_string()));
        assert_eq!(parse_bestmove_line("bestmove e2e4"), Some("e2e4".to_string()));
        assert_eq!(parse_bestmove_line("info depth 1"), None);
    }

    #[test]
    fn pv_to_san_replays_from_the_given_position() {
        let pv = vec!["e2e4".to_string(), "e7e5".to_string(), "g1f3".to_string()];
        let san = pv_to_san(&shakmaty::Chess::default(), &pv);
        assert_eq!(san, vec!["e4", "e5", "Nf3"]);
    }

    #[test]
    fn pv_to_san_stops_at_first_bad_move() {
        let pv = vec!["e2e4".to_string(), "e2e9".to_string(), "g1f3".to_string()];
        let san = pv_to_san(&shakmaty::Chess::default(), &pv);
        assert_eq!(san, vec!["e4"]);
    }

    #[test]
    fn formats_white_pov_scores() {
        assert_eq!(format_white_score(Score::Cp(32)), "+0.3");
        assert_eq!(format_white_score(Score::Cp(-150)), "-1.5");
        assert_eq!(format_white_score(Score::Mate(3)), "#3");
        assert_eq!(format_white_score(Score::Mate(-2)), "#-2");
    }

    #[test]
    fn white_fraction_is_half_at_equality() {
        assert_eq!(white_fraction(Score::Cp(0)), 0.5);
    }

    #[test]
    fn white_fraction_saturates_toward_extremes_without_reaching_them() {
        let big_white_edge = white_fraction(Score::Cp(3000));
        let big_black_edge = white_fraction(Score::Cp(-3000));
        assert!(big_white_edge > 0.99 && big_white_edge < 1.0);
        assert!(big_black_edge < 0.01 && big_black_edge > 0.0);
    }

    #[test]
    fn white_fraction_is_symmetric() {
        let a = white_fraction(Score::Cp(250));
        let b = white_fraction(Score::Cp(-250));
        assert!((a - (1.0 - b)).abs() < 1e-9);
    }

    #[test]
    fn white_fraction_saturates_fully_at_mate() {
        assert_eq!(white_fraction(Score::Mate(4)), 1.0);
        assert_eq!(white_fraction(Score::Mate(-4)), 0.0);
    }
}
