use serde::{Deserialize, Serialize};
use strum::EnumIter;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GameConfig {
    pub time_control: TimeControl,
    pub variant: Variant,
    pub rated: RatingMode,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, EnumIter)]
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

impl TimeControl {
    pub fn category(&self) -> Category {
        let seconds = self.initial_time / 1000;
        match seconds {
            ..120 => Category::Bullet,
            120..300 => Category::Blitz,
            300..1500 => Category::Rapid,
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

    #[test]
    fn category_bullet_under_120s() {
        let tc = TimeControl { initial_time: 119_000, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Bullet));
    }

    #[test]
    fn category_boundary_119999ms_is_bullet() {
        // 119_999 ms / 1000 = 119 s (integer division) → Bullet
        let tc = TimeControl { initial_time: 119_999, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Bullet));
    }

    #[test]
    fn category_boundary_120s_is_blitz() {
        let tc = TimeControl { initial_time: 120_000, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Blitz));
    }

    #[test]
    fn category_boundary_299s_is_blitz() {
        let tc = TimeControl { initial_time: 299_000, mode: TimeMode::Increment(0) };
        assert!(matches!(tc.category(), Category::Blitz));
    }

    #[test]
    fn category_boundary_300s_is_rapid() {
        let tc = TimeControl { initial_time: 300_000, mode: TimeMode::Increment(0) };
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
