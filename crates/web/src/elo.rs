fn k_factor(games: i32, rating: i32) -> f64 {
    if games < 30 {
        32.0
    } else if rating < 2300 {
        24.0
    } else {
        16.0
    }
}

pub fn new_rating(mine: i32, theirs: i32, score: f64, games_played: i32) -> i32 {
    let expected = 1.0 / (1.0 + 10f64.powf((theirs - mine) as f64 / 400.0));
    mine + (k_factor(games_played, mine) * (score - expected)).round() as i32
}
