//! Board appearance preferences, shared app-wide.
//!
//! # Why these are globals rather than context
//!
//! The first version of this provided a `BoardPrefs` struct via
//! `provide_context` at the App root, like `FriendsPresence` does. It did not
//! work: the provider's effect demonstrably wrote the right values (logged
//! `piece=anarcandy board=wood`), while every reader — the settings page's own
//! swatches included — still saw the defaults. Two distinct instances existed,
//! with writes going to one and reads coming from the other.
//!
//! A `thread_local!` global removes that failure mode by construction: there
//! is exactly one set of signals, and `use_context` isn't in the path at all.
//! That's sound here specifically because the client is single-threaded WASM
//! serving exactly one user. It would NOT be sound on the server, where one
//! process serves everybody — so under `ssr` the accessors hand back fresh
//! default signals per call instead. That's also the correct SSR behaviour:
//! the server has no business rendering one user's piece set, and defaults are
//! what the client's first paint uses too, so hydration matches.
//!
//! # Why not derive from the user resource directly
//!
//! Because every one of the 64 squares would then read that resource during
//! render, which is a hydration-mismatch hazard Leptos warns about at runtime.
//! These are plain signals seeded by a single effect instead.

use leptos::prelude::*;

/// Signals for the board's appearance.
#[derive(Copy, Clone)]
pub struct BoardPrefs {
    /// One of `shared::PIECE_SETS`. Interpolated into piece image URLs, so it
    /// is only ever written from a sanitized source.
    pub piece_set: RwSignal<&'static str>,
    /// One of `shared::BOARD_THEMES`. Mirrored onto `<html data-board>`, which
    /// is what the stylesheet's `--board-light`/`--board-dark` pairs key off.
    pub board_theme: RwSignal<&'static str>,
}

#[cfg(feature = "hydrate")]
thread_local! {
    /// The single client-side instance, installed by `provide_board_prefs`.
    ///
    /// Deliberately filled in there rather than created lazily here: a signal
    /// built on first access would belong to whichever component happened to
    /// touch it first, and Leptos disposes a signal with its owner — so it
    /// would silently go dead the moment that component unmounted, which is
    /// exactly the symptom this module's second bug produced (writes landing,
    /// reads returning defaults). Created inside `App`'s root owner, it lives
    /// as long as the app does.
    static PREFS: std::cell::Cell<Option<BoardPrefs>> = const { std::cell::Cell::new(None) };
}

#[cfg(feature = "hydrate")]
pub fn use_board_prefs() -> BoardPrefs {
    PREFS.with(|p| {
        p.get().expect("provide_board_prefs must be called at the App root")
    })
}

/// Under SSR every call gets its own defaults — see the module docs for why
/// sharing them across requests would be a cross-user leak.
#[cfg(not(feature = "hydrate"))]
pub fn use_board_prefs() -> BoardPrefs {
    BoardPrefs {
        piece_set: RwSignal::new(shared::DEFAULT_PIECE_SET),
        board_theme: RwSignal::new(shared::DEFAULT_BOARD_THEME),
    }
}

/// Resolves an arbitrary string to the matching `&'static str` from an
/// allowlist. Storing the static rather than an owned `String` keeps these
/// signals cheap to read from every square.
fn resolve(value: &str, allowed: &[(&'static str, &str)], fallback: &'static str) -> &'static str {
    allowed
        .iter()
        .find(|(id, _)| *id == value)
        .map(|(id, _)| *id)
        .unwrap_or(fallback)
}

pub fn resolve_piece_set(value: &str) -> &'static str {
    resolve(value, shared::PIECE_SETS, shared::DEFAULT_PIECE_SET)
}

pub fn resolve_board_theme(value: &str) -> &'static str {
    resolve(value, shared::BOARD_THEMES, shared::DEFAULT_BOARD_THEME)
}

/// Writes both preferences onto `<html>`, where the stylesheet reads them.
///
/// Public so the settings page can apply a change optimistically, without
/// waiting for the save to round-trip and the user resource to refetch.
#[cfg(feature = "hydrate")]
pub fn apply_board_attrs(board_theme: &str, piece_set: &str) {
    if let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = root.set_attribute("data-board", board_theme);
        let _ = root.set_attribute("data-pieces", piece_set);
    }
}

/// Call once at the App root, after `provide_current_user()`.
pub fn provide_board_prefs() {
    #[cfg(feature = "hydrate")]
    {
        use crate::components::auth::use_current_user;

        // Created here, in `App`'s own owner, so they outlive every page.
        let prefs = BoardPrefs {
            piece_set: RwSignal::new(shared::DEFAULT_PIECE_SET),
            board_theme: RwSignal::new(shared::DEFAULT_BOARD_THEME),
        };
        PREFS.with(|p| p.set(Some(prefs)));

        // Seed from the signed-in user's stored choices. Re-runs when the auth
        // trigger bumps (the settings page saving), so a change lands on every
        // open board without a reload.
        Effect::new(move |_| {
            let Some(Ok(Some(user))) = use_current_user().get() else { return };
            prefs.piece_set.set(resolve_piece_set(&user.settings.piece_set));
            prefs.board_theme.set(resolve_board_theme(&user.settings.board_theme));
        });

        // Mirror both preferences onto <html>, where the stylesheet reads
        // them: `data-board` selects the square colours, `data-pieces` the
        // piece images. A separate effect from the seeding one above so it
        // also fires for the settings page's optimistic update, before the
        // server round trip.
        //
        // The DOM is deliberately the transport for both. Driving the pieces
        // from a signal read inside `Square` did not work — see the piece-set
        // block in main.css.
        Effect::new(move |_| {
            apply_board_attrs(prefs.board_theme.get(), prefs.piece_set.get());
        });
    }
}
