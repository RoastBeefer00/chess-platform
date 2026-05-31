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
