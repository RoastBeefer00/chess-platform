use serde::{Deserialize, Serialize};

/// Selectable board colour schemes. The values are the `data-board` attribute
/// the stylesheet keys its `--board-light`/`--board-dark` pairs off — see the
/// board-theme block in `style/main.css`.
///
/// An allowlist rather than a free string because these values reach the DOM.
/// `piece_set` in particular is interpolated into an image URL, where an
/// arbitrary string would be a path-traversal hole; both are modelled the
/// same way so neither can drift into one.
pub const BOARD_THEMES: &[(&str, &str)] = &[
    ("green", "Green"),
    ("brown", "Brown"),
    ("blue", "Blue"),
    ("purple", "Purple"),
    ("grey", "Grey"),
    ("wood", "Wood"),
];

pub const DEFAULT_BOARD_THEME: &str = "green";

/// Piece sets vendored under `public/piece/<name>/`. Not the full 40 that ship
/// in that directory — this is a curated subset, since a picker with forty
/// near-identical options is worse than one with a dozen good ones.
pub const PIECE_SETS: &[(&str, &str)] = &[
    ("alpha", "Alpha"),
    ("cburnett", "Cburnett"),
    ("merida", "Merida"),
    ("maestro", "Maestro"),
    ("fresca", "Fresca"),
    ("gioco", "Gioco"),
    ("governor", "Governor"),
    ("dubrovny", "Dubrovny"),
    ("staunty", "Staunty"),
    ("horsey", "Horsey"),
    ("anarcandy", "Anarcandy"),
    ("celtic", "Celtic"),
];

pub const DEFAULT_PIECE_SET: &str = "alpha";

/// `value` if it appears in `allowed`, otherwise `fallback`.
///
/// Every path that takes one of these from the client runs it through here.
/// The server function validates on write so bad data never lands in the DB,
/// and the render path validates again on read so a row written by an older
/// build (or edited by hand) still can't inject a path into an image URL.
fn sanitize(value: &str, allowed: &[(&str, &str)], fallback: &'static str) -> String {
    if allowed.iter().any(|(id, _)| *id == value) {
        value.to_string()
    } else {
        fallback.to_string()
    }
}

/// A user's stored preferences. Persisted as JSONB (`users.settings`) rather
/// than typed columns, so every field is `#[serde(default)]` — a key absent
/// from an old row, or from a fresh `'{}'`, just falls back to its default
/// instead of failing to deserialize. Add new fields the same way; no
/// migration needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserSettings {
    /// Skip the promotion-role picker and always promote to queen.
    pub auto_queen: bool,
    /// Board colour scheme; one of `BOARD_THEMES`.
    pub board_theme: String,
    /// Piece set; one of `PIECE_SETS`.
    pub piece_set: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            auto_queen: false,
            board_theme: DEFAULT_BOARD_THEME.to_string(),
            piece_set: DEFAULT_PIECE_SET.to_string(),
        }
    }
}

impl UserSettings {
    /// Board theme, guaranteed to be one of `BOARD_THEMES`.
    pub fn board_theme(&self) -> String {
        sanitize(&self.board_theme, BOARD_THEMES, DEFAULT_BOARD_THEME)
    }

    /// Piece set, guaranteed to be one of `PIECE_SETS`. Use this — never the
    /// raw field — anywhere the value becomes part of a URL.
    pub fn piece_set(&self) -> String {
        sanitize(&self.piece_set, PIECE_SETS, DEFAULT_PIECE_SET)
    }

    /// Drops anything not on the allowlists back to its default. Called by
    /// `update_settings` before persisting.
    pub fn sanitized(self) -> Self {
        Self {
            auto_queen: self.auto_queen,
            board_theme: self.board_theme(),
            piece_set: self.piece_set(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let settings: UserSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, UserSettings::default());
        assert_eq!(settings.board_theme(), "green");
        assert_eq!(settings.piece_set(), "alpha");
    }

    #[test]
    fn unknown_future_key_is_ignored_not_rejected() {
        let settings: UserSettings =
            serde_json::from_str(r#"{"auto_queen": true, "future_thing": "x"}"#).unwrap();
        assert!(settings.auto_queen);
    }

    #[test]
    fn known_values_round_trip() {
        let settings = UserSettings {
            auto_queen: true,
            board_theme: "brown".to_string(),
            piece_set: "merida".to_string(),
        };
        assert_eq!(settings.clone().sanitized(), settings);
    }

    /// The whole reason these are allowlists: `piece_set` is interpolated
    /// into `/piece/<set>/wP.svg`, so an arbitrary value would be a path
    /// traversal.
    #[test]
    fn traversal_and_junk_are_rejected() {
        let settings = UserSettings {
            auto_queen: false,
            board_theme: "'; drop table users--".to_string(),
            piece_set: "../../../etc/passwd".to_string(),
        };
        assert_eq!(settings.board_theme(), "green");
        assert_eq!(settings.piece_set(), "alpha");

        let cleaned = settings.sanitized();
        assert_eq!(cleaned.board_theme, "green");
        assert_eq!(cleaned.piece_set, "alpha");
    }

    /// Every advertised option must actually exist as a directory under
    /// `public/piece/`, or the picker offers a set that renders as broken
    /// images. Checked here rather than trusted, because the list was
    /// hand-curated from a directory of 40.
    #[test]
    fn every_listed_piece_set_is_vendored() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../web/public/piece");
        for (id, _) in PIECE_SETS {
            let dir = std::path::Path::new(root).join(id);
            assert!(dir.is_dir(), "piece set {id:?} is listed but {dir:?} does not exist");
            for piece in ["wK", "wQ", "wR", "wB", "wN", "wP", "bK", "bQ", "bR", "bB", "bN", "bP"] {
                let svg = dir.join(format!("{piece}.svg"));
                assert!(svg.is_file(), "piece set {id:?} is missing {piece}.svg");
            }
        }
    }
}
