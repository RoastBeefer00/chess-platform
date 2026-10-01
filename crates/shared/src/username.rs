//! Username rules.
//!
//! Lives in `shared` so the onboarding form's live feedback and the
//! `set_username` server function's authoritative check are literally the
//! same code — a server function is a plain HTTP endpoint, so client-side
//! validation is a convenience, never a control.

pub const USERNAME_MIN_LEN: usize = 5;
pub const USERNAME_MAX_LEN: usize = 32;

/// `Ok(())` if `username` is legal, otherwise a message fit to show the user
/// directly.
///
/// ASCII-only is deliberate rather than lazy: usernames are compared
/// case-insensitively for uniqueness (see the `users_username_lower_unique_idx`
/// migration), and unicode case folding plus homoglyphs would make
/// "distinct" names that render identically.
pub fn validate_username(username: &str) -> Result<(), &'static str> {
    if username.len() < USERNAME_MIN_LEN {
        return Err("Username must be at least 5 characters");
    }
    if username.len() > USERNAME_MAX_LEN {
        return Err("Username must be at most 32 characters");
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err("Username may only contain letters, numbers, and underscores");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_names() {
        assert!(validate_username("magnus").is_ok());
        assert!(validate_username("Player_1").is_ok());
        assert!(validate_username("a1_Z9").is_ok());
    }

    #[test]
    fn rejects_too_short_and_too_long() {
        assert!(validate_username("").is_err());
        assert!(validate_username("abcd").is_err());
        assert!(validate_username(&"a".repeat(33)).is_err());
        assert!(validate_username(&"a".repeat(32)).is_ok());
    }

    #[test]
    fn rejects_non_ascii_and_punctuation() {
        assert!(validate_username("magn us").is_err());
        assert!(validate_username("magnus!").is_err());
        assert!(validate_username("magnuß").is_err());
        // Cyrillic 'а' renders like ASCII 'a' — exactly the homoglyph case
        // the ASCII restriction exists to rule out.
        assert!(validate_username("mаgnus").is_err());
    }
}
