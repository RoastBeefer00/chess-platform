use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum GameMode {
    Bullet,
    Blitz,
    Rapid,
    Classical,
    NineSixty,
}

impl std::fmt::Display for GameMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            GameMode::Bullet => "bullet",
            GameMode::Blitz => "blitz",
            GameMode::Rapid => "rapid",
            GameMode::Classical => "classical",
            GameMode::NineSixty => "960",
        };
        f.write_str(s)
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Bucket {
    // Bullet
    Bullet30,
    Bullet60,
    Bullet60Plus1,
    Bullet120Plus1,
    // Blitz
    Blitz180,
    Blitz180Plus2,
    Blitz300,
    Blitz300Plus3,
    // Rapid
    Rapid600,
    Rapid600Plus5,
    Rapid900Plus10,
    // Classical
    Classical1800,
    Classical1800Plus20,
    Classical3600,
    // 960 (Chess960 / Fischer Random) — bucket separately so variant players don't pair with standard
    NineSixty300,
    NineSixty300Plus3,
    NineSixty600,
    NineSixty600Plus5,
}

impl Bucket {
    pub fn id(&self, rating_mode: RatingMode) -> String {
        format!("{}:{}", self, rating_mode)
    }

    pub fn mode(&self) -> GameMode {
        match self {
            Bucket::Bullet30
            | Bucket::Bullet60
            | Bucket::Bullet60Plus1
            | Bucket::Bullet120Plus1 => GameMode::Bullet,
            Bucket::Blitz180 | Bucket::Blitz180Plus2 | Bucket::Blitz300 | Bucket::Blitz300Plus3 => {
                GameMode::Blitz
            }
            Bucket::Rapid600 | Bucket::Rapid600Plus5 | Bucket::Rapid900Plus10 => GameMode::Rapid,
            Bucket::Classical1800 | Bucket::Classical1800Plus20 | Bucket::Classical3600 => {
                GameMode::Classical
            }
            Bucket::NineSixty300
            | Bucket::NineSixty300Plus3
            | Bucket::NineSixty600
            | Bucket::NineSixty600Plus5 => GameMode::NineSixty,
        }
    }
}

impl std::fmt::Display for Bucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Bucket::Bullet30 => "mm:30+0",
            Bucket::Bullet60 => "mm:60+0",
            Bucket::Bullet60Plus1 => "mm:60+1",
            Bucket::Bullet120Plus1 => "mm:120+1",
            Bucket::Blitz180 => "mm:180+0",
            Bucket::Blitz180Plus2 => "mm:180+2",
            Bucket::Blitz300 => "mm:300+0",
            Bucket::Blitz300Plus3 => "mm:300+3",
            Bucket::Rapid600 => "mm:600+0",
            Bucket::Rapid600Plus5 => "mm:600+5",
            Bucket::Rapid900Plus10 => "mm:900+10",
            Bucket::Classical1800 => "mm:1800+0",
            Bucket::Classical1800Plus20 => "mm:1800+20",
            Bucket::Classical3600 => "mm:3600+0",
            Bucket::NineSixty300 => "mm:960:300+0",
            Bucket::NineSixty300Plus3 => "mm:960:300+3",
            Bucket::NineSixty600 => "mm:960:600+0",
            Bucket::NineSixty600Plus5 => "mm:960:600+5",
        };
        f.write_str(s)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum RatingMode {
    Casual,
    Rated,
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
