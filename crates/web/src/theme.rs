//! Colour-theme selection.
//!
//! The actual theming is pure CSS — see the `[data-theme="light"]` block in
//! `style/main.css`. This module only decides *which* value that attribute
//! carries.

/// Where the preference lives. Per-device rather than on the user's account:
/// a theme is a property of the screen you're looking at, and this also means
/// it applies to signed-out visitors, who have no account to store it on.
/// Same reasoning (and same storage) as `PlayHub`'s rating-mode memory.
pub const THEME_STORAGE_KEY: &str = "gambit:theme";

/// The three states of the settings control. `System` is the default and
/// means "follow `prefers-color-scheme`".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "light" => ThemeChoice::Light,
            "dark" => ThemeChoice::Dark,
            _ => ThemeChoice::System,
        }
    }
}

/// Runs before first paint, inlined into the document `<head>`.
///
/// This has to be a blocking inline script rather than anything the WASM
/// bundle does: by the time hydration runs the page has already painted, so
/// resolving the theme there would show every visitor a flash of the dark
/// theme before switching. It writes the same `data-theme` attribute that
/// `apply_theme` writes later, so the two can't disagree.
///
/// Kept deliberately tiny and dependency-free — it is the one piece of
/// hand-written JS in the project.
pub const THEME_BOOTSTRAP_JS: &str = r#"
(function () {
  try {
    var stored = localStorage.getItem('gambit:theme');
    var dark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    var theme = stored === 'light' || stored === 'dark'
      ? stored
      : (dark ? 'dark' : 'light');
    document.documentElement.setAttribute('data-theme', theme);
  } catch (e) {
    document.documentElement.setAttribute('data-theme', 'dark');
  }
})();
"#;

/// Reads the stored choice. `System` when absent or unparseable.
#[cfg(feature = "hydrate")]
pub fn stored_choice() -> ThemeChoice {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(THEME_STORAGE_KEY).ok().flatten())
        .map(|v| ThemeChoice::from_str(&v))
        .unwrap_or(ThemeChoice::System)
}

/// Persists `choice` and applies it to the live document immediately.
#[cfg(feature = "hydrate")]
pub fn set_choice(choice: ThemeChoice) {
    let Some(window) = web_sys::window() else { return };
    if let Ok(Some(storage)) = window.local_storage() {
        let _ = storage.set_item(THEME_STORAGE_KEY, choice.as_str());
    }
    apply_theme(choice);
}

/// Resolves `choice` against the system preference and writes `data-theme`.
#[cfg(feature = "hydrate")]
pub fn apply_theme(choice: ThemeChoice) {
    let Some(window) = web_sys::window() else { return };
    let resolved = match choice {
        ThemeChoice::Light => "light",
        ThemeChoice::Dark => "dark",
        ThemeChoice::System => {
            let prefers_dark = window
                .match_media("(prefers-color-scheme: dark)")
                .ok()
                .flatten()
                .is_some_and(|m| m.matches());
            if prefers_dark { "dark" } else { "light" }
        }
    };
    if let Some(root) = window.document().and_then(|d| d.document_element()) {
        let _ = root.set_attribute("data-theme", resolved);
    }
}
