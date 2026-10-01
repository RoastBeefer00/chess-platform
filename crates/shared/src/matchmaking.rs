use serde::{Deserialize, Serialize};
use strum::EnumIter;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GameConfig {
    pub time_control: TimeControl,
    pub variant: Variant,
    pub rated: RatingMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize, EnumIter)]
pub enum Category {
    Bullet,
    Blitz,
    Rapid,
    Classical,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Variant {
    Standard,
    Chess960,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TimeControl {
    pub initial_time: i64,
    pub mode: TimeMode,
}

impl Category {
    /// Parses the lowercase string stored in `games.mode` / `ratings.mode`
    /// back into a `Category`. `None` for a mode that isn't one of these
    /// four — `ratings` also carries '960' and 'puzzle' rows, which are real
    /// pools but not game categories.
    ///
    /// The inverse of the `Display` impl below.
    pub fn from_mode(mode: &str) -> Option<Self> {
        match mode {
            "bullet" => Some(Category::Bullet),
            "blitz" => Some(Category::Blitz),
            "rapid" => Some(Category::Rapid),
            "classical" => Some(Category::Classical),
            _ => None,
        }
    }
}

impl TimeControl {
    /// Which rating pool a game counts toward.
    ///
    /// Decided by the initial time alone — the increment is deliberately not
    /// part of it, so 5+0 and 5+5 are the same category, as the play hub's
    /// own grouping already says they are.
    ///
    /// The boundaries follow the usual convention (bullet under 3 minutes,
    /// blitz under 10) and, importantly, agree with how `PlayHub` groups the
    /// presets it offers. They did not before: 5+0 and 5+5 both sat under
    /// "Blitz" in the UI while updating the *Rapid* rating, and 2+1 sat under
    /// "Bullet" while updating Blitz. Any change here has to keep
    /// `category_matches_play_hub_grouping` below passing.
    pub fn category(&self) -> Category {
        let seconds = self.initial_time / 1000;
        match seconds {
            ..180 => Category::Bullet,
            180..600 => Category::Blitz,
            600..1500 => Category::Rapid,
            _ => Category::Classical,
        }
    }

    pub fn bucket(&self, rated: RatingMode) -> String {
        let seconds = self.initial_time / 1000;
        match self.mode {
            TimeMode::Increment(i) => format!("mm:{}+{}:i:{}", seconds, i, rated),
            TimeMode::Delay(d) => format!("mm:{}+{}:d:{}", seconds, d, rated),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum TimeMode {
    Increment(i64),
    Delay(i64),
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Category::Bullet => "bullet",
            Category::Blitz => "blitz",
            Category::Rapid => "rapid",
            Category::Classical => "classical",
        };
        f.write_str(s)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum RatingMode {
    Casual,
    Rated,
}

impl RatingMode {
    pub fn is_rated(&self) -> bool {
        match self {
            RatingMode::Casual => false,
            RatingMode::Rated => true,
        }
    }
}

impl std::fmt::Display for RatingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RatingMode::Casual => "casual",
            RatingMode::Rated => "rated",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TimeControl::category ────────────────────────────────────────────────
    //
    // Boundaries are on the INITIAL time only; the increment never moves a
    // game between pools.

    #[test]
    fn category_bullet_under_180s() {
        let tc = TimeControl { initial_time: 119_000, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Bullet));
    }

    #[test]
    fn category_boundary_179999ms_is_bullet() {
        // 179_999 ms / 1000 = 179 s (integer division) → Bullet
        let tc = TimeControl { initial_time: 179_999, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Bullet));
    }

    #[test]
    fn category_boundary_180s_is_blitz() {
        let tc = TimeControl { initial_time: 180_000, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Blitz));
    }

    #[test]
    fn category_boundary_599s_is_blitz() {
        let tc = TimeControl { initial_time: 599_000, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Blitz));
    }

    #[test]
    fn category_boundary_600s_is_rapid() {
        let tc = TimeControl { initial_time: 600_000, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Rapid));
    }

    #[test]
    fn category_boundary_1499s_is_rapid() {
        let tc = TimeControl { initial_time: 1_499_000, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Rapid));
    }

    #[test]
    fn category_boundary_1500s_is_classical() {
        let tc = TimeControl { initial_time: 1_500_000, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Classical));
    }

    /// The increment must not move a game between rating pools. This is the
    /// bug that prompted the boundary change: 5+5 updated the Rapid rating
    /// while the UI offered it as Blitz.
    #[test]
    fn category_ignores_the_increment() {
        for inc in [0, 1_000, 2_000, 5_000, 10_000, 60_000] {
            let tc = TimeControl { initial_time: 300_000, mode: TimeMode::Increment(inc) };
            assert!(
                matches!(tc.category(), Category::Blitz),
                "5+{}s should be Blitz, same as 5+0",
                inc / 1000
            );
        }
        // Same for a delay-based control.
        let tc = TimeControl { initial_time: 300_000, mode: TimeMode::Delay(10_000) };
        assert!(matches!(tc.category(), Category::Blitz));
    }

    /// Every preset the play hub offers must land in the pool the hub files
    /// it under. Keep this in sync with `PlayHub`'s BULLET/BLITZ/RAPID lists —
    /// the two disagreeing is exactly how 5+0, 5+5 and 2+1 ended up updating
    /// the wrong rating.
    #[test]
    fn category_matches_play_hub_grouping() {
        let presets: &[(&str, i64, i64, Category)] = &[
            ("1 + 0", 60_000, 0, Category::Bullet),
            ("1 + 1", 60_000, 1_000, Category::Bullet),
            ("2 + 1", 120_000, 1_000, Category::Bullet),
            ("3 + 0", 180_000, 0, Category::Blitz),
            ("3 + 2", 180_000, 2_000, Category::Blitz),
            ("5 + 0", 300_000, 0, Category::Blitz),
            ("5 + 5", 300_000, 5_000, Category::Blitz),
            ("10 + 0", 600_000, 0, Category::Rapid),
            ("15 + 10", 900_000, 10_000, Category::Rapid),
        ];
        for (label, initial, inc, expected) in presets {
            let tc = TimeControl { initial_time: *initial, mode: TimeMode::Increment(*inc) };
            assert_eq!(
                tc.category().to_string(),
                expected.to_string(),
                "{label} should be {expected}, got {}",
                tc.category()
            );
        }
    }

    // ── TimeControl::bucket ──────────────────────────────────────────────────

    #[test]
    fn bucket_increment_casual() {
        // increment value is stored in ms and emitted as-is
        let tc = TimeControl { initial_time: 300_000, mode: TimeMode::Increment(5_000) };
        assert_eq!(tc.bucket(RatingMode::Casual), "mm:300+5000:i:casual");
    }

    #[test]
    fn bucket_increment_rated() {
        let tc = TimeControl { initial_time: 600_000, mode: TimeMode::Increment(0) };
        assert_eq!(tc.bucket(RatingMode::Rated), "mm:600+0:i:rated");
    }

    #[test]
    fn bucket_delay_casual() {
        let tc = TimeControl { initial_time: 180_000, mode: TimeMode::Delay(2_000) };
        assert_eq!(tc.bucket(RatingMode::Casual), "mm:180+2000:d:casual");
    }

    // ── RatingMode ───────────────────────────────────────────────────────────

    #[test]
    fn rating_mode_is_rated() {
        assert!(RatingMode::Rated.is_rated());
        assert!(!RatingMode::Casual.is_rated());
    }

    #[test]
    fn rating_mode_display() {
        assert_eq!(RatingMode::Rated.to_string(), "rated");
        assert_eq!(RatingMode::Casual.to_string(), "casual");
    }

    // ── Category display ─────────────────────────────────────────────────────

    #[test]
    fn category_display() {
        assert_eq!(Category::Bullet.to_string(), "bullet");
        assert_eq!(Category::Blitz.to_string(), "blitz");
        assert_eq!(Category::Rapid.to_string(), "rapid");
        assert_eq!(Category::Classical.to_string(), "classical");
    }
}
