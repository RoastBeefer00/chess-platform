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

#[cfg(test)]
mod tests {
    use super::*;

    // ── k_factor tiers ───────────────────────────────────────────────────────
    // At equal ratings, expected = 0.5; win gain = K * 0.5 = K/2.

    #[test]
    fn k_factor_under_30_games_is_32() {
        // 29 games → K=32; equal-rating win → +16
        let after = new_rating(1500, 1500, 1.0, 29);
        assert_eq!(after - 1500, 16, "K=32 → +16 for equal-rating win");
    }

    #[test]
    fn k_factor_30_plus_games_below_2300_is_24() {
        // 30 games, rating < 2300 → K=24; equal-rating win → +12
        let after = new_rating(1500, 1500, 1.0, 30);
        assert_eq!(after - 1500, 12, "K=24 → +12 for equal-rating win");
    }

    #[test]
    fn k_factor_30_plus_games_at_or_above_2300_is_16() {
        // 100 games, rating ≥ 2300 → K=16; equal-rating win → +8
        let after = new_rating(2300, 2300, 1.0, 100);
        assert_eq!(after - 2300, 8, "K=16 → +8 for equal-rating win");
    }

    // ── outcome symmetry ─────────────────────────────────────────────────────

    #[test]
    fn draw_at_equal_rating_no_change() {
        let after = new_rating(1500, 1500, 0.5, 100);
        assert_eq!(after, 1500, "draw between equals → no rating change");
    }

    #[test]
    fn win_loss_symmetric_at_equal_rating() {
        let winner_gain = new_rating(1500, 1500, 1.0, 100) - 1500;
        let loser_loss = 1500 - new_rating(1500, 1500, 0.0, 100);
        assert_eq!(winner_gain, loser_loss, "win/loss should be symmetric");
    }

    #[test]
    fn favourite_gains_less_than_underdog() {
        // High-rated player beating a low-rated: small gain.
        let fav_gain = new_rating(2000, 1500, 1.0, 100) - 2000;
        // Low-rated player beating the favourite: large gain.
        let upset_gain = new_rating(1500, 2000, 1.0, 100) - 1500;
        assert!(
            fav_gain < upset_gain,
            "favourite gain ({fav_gain}) should be less than upset gain ({upset_gain})"
        );
        assert!(fav_gain >= 0, "favourite still gains something");
    }

    #[test]
    fn underdog_loses_less_than_favourite_wins() {
        // When favourite wins, underdog loses only a small amount.
        let underdog_loss = 1500 - new_rating(1500, 2000, 0.0, 100);
        let fav_gain = new_rating(2000, 1500, 1.0, 100) - 2000;
        assert_eq!(
            underdog_loss, fav_gain,
            "zero-sum: what favourite gains = what underdog loses"
        );
    }
}
