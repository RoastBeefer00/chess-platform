use serde::{Deserialize, Serialize};

/// A user's stored preferences. Persisted as JSONB (`users.settings`) rather
/// than typed columns, so every field is `#[serde(default)]` — a key absent
/// from an old row, or from a fresh `'{}'`, just falls back to its default
/// instead of failing to deserialize. Add new fields the same way; no
/// migration needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UserSettings {
    /// Skip the promotion-role picker and always promote to queen.
    pub auto_queen: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let settings: UserSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, UserSettings::default());
    }

    #[test]
    fn unknown_future_key_is_ignored_not_rejected() {
        let settings: UserSettings =
            serde_json::from_str(r#"{"auto_queen": true, "board_theme": "walnut"}"#).unwrap();
        assert!(settings.auto_queen);
    }
}
