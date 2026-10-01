use leptos::prelude::*;
use shared::UserSettings;

use crate::components::auth::{use_auth_trigger, use_current_user};

/// Reads the signed-in user's auto-queen preference. `get_untracked` since
/// call sites (a click/drop handler) run once per gesture, not reactively —
/// same style as the `is_my_turn`/`position` reads next to them.
#[cfg(feature = "hydrate")]
pub(crate) fn auto_queen_enabled() -> bool {
    use_current_user()
        .get_untracked()
        .and_then(|r| r.ok())
        .flatten()
        .is_some_and(|u| u.settings.auto_queen)
}

#[server]
pub async fn update_settings(settings: UserSettings) -> Result<(), ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let Some(user_id) = auth.user.as_ref().map(|u| u.id) else {
        return Err(ServerFnError::ServerError("not signed in".to_string()));
    };
    let state = expect_context::<AppState>();
    // Never trust the posted values: `piece_set` becomes part of an image
    // URL, so anything off the allowlist is dropped back to its default
    // before it can reach the DB. The render path sanitizes again on read.
    let settings = settings.sanitized();
    Ok(state.user_store.update_settings(user_id, &settings).await?)
}

/// Light / dark / follow-the-system picker.
///
/// Reads and writes `localStorage` directly rather than going through
/// `update_settings`: a theme belongs to the device you're looking at, not
/// the account, and this way it works for signed-out visitors too. The
/// initial read happens in an `Effect` for the usual reason — there is no
/// `localStorage` during SSR, so reading at render time would guarantee a
/// hydration mismatch.
#[component]
fn ThemePicker() -> impl IntoView {
    use crate::theme::ThemeChoice;

    let choice = RwSignal::new(ThemeChoice::System);

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| choice.set(crate::theme::stored_choice()));

    let select = move |next: ThemeChoice| {
        choice.set(next);
        #[cfg(feature = "hydrate")]
        crate::theme::set_choice(next);
    };

    let option = move |value: ThemeChoice, label: &'static str| {
        view! {
            <button
                type="button"
                role="radio"
                aria-checked=move || (choice.get() == value).to_string()
                on:click=move |_| select(value)
                class="px-3 py-1.5 text-xs font-semibold uppercase tracking-wide rounded-control transition-colors cursor-pointer"
                class:bg-white=move || choice.get() == value
                class:text-zinc-950=move || choice.get() == value
                class:text-zinc-400=move || choice.get() != value
            >
                {label}
            </button>
        }
    };

    view! {
        <div class="flex items-center rounded-control bg-zinc-950 border border-zinc-800/60 p-0.5" role="radiogroup" aria-label="Colour theme">
            {option(ThemeChoice::System, "System")}
            {option(ThemeChoice::Light, "Light")}
            {option(ThemeChoice::Dark, "Dark")}
        </div>
    }
}

#[component]
fn Toggle(checked: Signal<bool>, on_toggle: Callback<()>) -> impl IntoView {
    view! {
        <button
            type="button"
            role="switch"
            aria-checked=move || checked.get().to_string()
            on:click=move |_| on_toggle.run(())
            class="relative w-11 h-6 rounded-full transition-colors cursor-pointer flex-shrink-0"
            class:bg-emerald-500=move || checked.get()
            class:bg-zinc-700=move || !checked.get()
        >
            <span
                class="absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white transition-transform"
                class:translate-x-5=move || checked.get()
            />
        </button>
    }
}

/// Settings page. Reads the current user's `UserSettings` off the shared
/// `use_current_user` resource (already fetched app-wide) and writes changes
/// back through `update_settings`, then bumps the auth trigger so every
/// consumer of `use_current_user` (including the game board's auto-queen
/// check) picks up the new value on its next read.
/// A single board-theme swatch: two squares in that theme's own colours,
/// so the choice is previewed rather than named.
#[component]
fn BoardSwatch(theme: &'static str, selected: Signal<bool>, on_pick: Callback<&'static str>) -> impl IntoView {
    view! {
        <button
            type="button"
            role="radio"
            aria-checked=move || selected.get().to_string()
            aria-label=theme
            on:click=move |_| on_pick.run(theme)
            data-board=theme
            class="w-10 h-10 rounded-control overflow-hidden grid grid-cols-2 grid-rows-2 cursor-pointer
                   border-2 transition-colors"
            class:border-transparent=move || !selected.get()
            class:hover:border-zinc-700=move || !selected.get()
            class:accent-ring=move || selected.get()
        >
            // `data-board` on the button itself re-points the board tokens
            // for this subtree, so each swatch paints in its own theme.
            <span class="sq-light"></span>
            <span class="sq-dark"></span>
            <span class="sq-dark"></span>
            <span class="sq-light"></span>
        </button>
    }
}

/// A single piece-set option, previewed with that set's own knight.
#[component]
fn PieceSwatch(set: &'static str, label: &'static str, selected: Signal<bool>, on_pick: Callback<&'static str>) -> impl IntoView {
    view! {
        <button
            type="button"
            role="radio"
            aria-checked=move || selected.get().to_string()
            aria-label=label
            title=label
            on:click=move |_| on_pick.run(set)
            class="w-12 h-12 rounded-control flex items-center justify-center cursor-pointer
                   bg-zinc-950 border-2 transition-colors"
            class:border-zinc-800=move || !selected.get()
            class:hover:border-zinc-700=move || !selected.get()
            class:accent-ring=move || selected.get()
        >
            <img src=format!("/piece/{set}/wN.svg") alt="" class="w-9 h-9"/>
        </button>
    }
}

/// Settings page. Reads the current user's `UserSettings` off the shared
/// `use_current_user` resource (already fetched app-wide) and writes changes
/// back through `update_settings`, then bumps the auth trigger so every
/// consumer of `use_current_user` picks up the new value on its next read.
///
/// Board and piece choices additionally write straight into `BoardPrefs`, so
/// a live game repaints the instant you click rather than after the round
/// trip — the same optimistic pattern as the auto-queen toggle below.
#[component]
pub fn Settings() -> impl IntoView {
    use crate::board_prefs::{resolve_board_theme, resolve_piece_set};

    let user = use_current_user();
    let auth_trigger = use_auth_trigger();
    let server_settings = move || {
        user.get()
            .and_then(|r| r.ok())
            .flatten()
            .map(|u| u.settings)
            .unwrap_or_default()
    };
    let server_auto_queen = move || server_settings().auto_queen;

    // Deliberately NOT read from the app-wide `BoardPrefs` signals. Those
    // are written by an effect in the root chunk and proved unreliable to
    // read back from a lazily-loaded route — the swatches showed the default
    // selection while the stored value was something else. This page owns its
    // own optimistic state and reads the server value underneath it, which
    // needs no cross-chunk sharing at all.
    let board_override = RwSignal::new(None::<&'static str>);
    let piece_override = RwSignal::new(None::<&'static str>);
    let current_board = move || {
        board_override
            .get()
            .unwrap_or_else(|| resolve_board_theme(&server_settings().board_theme))
    };
    let current_pieces = move || {
        piece_override
            .get()
            .unwrap_or_else(|| resolve_piece_set(&server_settings().piece_set))
    };


    // Optimistic override for the toggle below: `Some(v)` while a save is in
    // flight (or has just succeeded), so the switch flips the instant it's
    // clicked instead of waiting on `update_settings` plus the
    // `current_user` refetch it triggers. Cleared once the server-confirmed
    // value catches up to the override, or reset to `None` (falling back to
    // the pre-toggle server value) if the save fails. Any future settings
    // toggle should follow this same three-piece shape: an override signal,
    // an effect that clears it once confirmed, and a read that prefers it.
    let auto_queen_override = RwSignal::new(None::<bool>);

    let save = Action::new(move |settings: &UserSettings| {
        let settings = settings.clone();
        async move {
            let result = update_settings(settings).await;
            match &result {
                Ok(()) => auth_trigger.0.update(|v| *v += 1),
                Err(_) => auto_queen_override.set(None),
            }
            result
        }
    });

    Effect::new(move |_| {
        if auto_queen_override.get().is_some_and(|pending| pending == server_auto_queen()) {
            auto_queen_override.set(None);
        }
    });

    let auto_queen = move || auto_queen_override.get().unwrap_or_else(server_auto_queen);

    // Builds the full settings payload from the live signals, so saving one
    // preference never clobbers another that changed in the same session.
    let current_payload = move || UserSettings {
        auto_queen: auto_queen(),
        board_theme: current_board().to_string(),
        piece_set: current_pieces().to_string(),
    };

    let toggle_auto_queen = move |_: ()| {
        let new_value = !auto_queen();
        auto_queen_override.set(Some(new_value));
        let mut payload = current_payload();
        payload.auto_queen = new_value;
        save.dispatch(payload);
    };

    // Each pick paints immediately (DOM attributes, the same route the root
    // effect uses) and then persists.
    let pick_board = Callback::new(move |theme: &str| {
        let theme = resolve_board_theme(theme);
        board_override.set(Some(theme));
        #[cfg(feature = "hydrate")]
        crate::board_prefs::apply_board_attrs(theme, current_pieces());
        let mut payload = current_payload();
        payload.board_theme = theme.to_string();
        save.dispatch(payload);
    });

    let pick_pieces = Callback::new(move |set: &str| {
        let set = resolve_piece_set(set);
        piece_override.set(Some(set));
        #[cfg(feature = "hydrate")]
        crate::board_prefs::apply_board_attrs(current_board(), set);
        let mut payload = current_payload();
        payload.piece_set = set.to_string();
        save.dispatch(payload);
    });

    view! {
        <div class="flex flex-col divide-y divide-zinc-800 surface-card overflow-hidden">
            <div class="flex items-center justify-between gap-4 px-4 py-4">
                <div class="flex flex-col gap-0.5 min-w-0">
                    <span class="text-sm font-semibold text-white">"Auto-queen"</span>
                    <span class="text-xs text-zinc-500">
                        "Always promote pawns to a queen, skipping the picker."
                    </span>
                </div>
                <Toggle checked=Signal::derive(auto_queen) on_toggle=Callback::new(toggle_auto_queen) />
            </div>

            <div class="flex items-center justify-between gap-4 px-4 py-4">
                <div class="flex flex-col gap-0.5 min-w-0">
                    <span class="text-sm font-semibold text-white">"Theme"</span>
                    <span class="text-xs text-zinc-500">
                        "Stored on this device, so it applies whether or not you're signed in."
                    </span>
                </div>
                <ThemePicker/>
            </div>

            <div class="flex flex-col gap-3 px-4 py-4">
                <div class="flex flex-col gap-0.5">
                    <span class="text-sm font-semibold text-white">"Board"</span>
                    <span class="text-xs text-zinc-500">
                        "Square colours. Independent of the light/dark theme."
                    </span>
                </div>
                <div class="flex flex-wrap gap-2" role="radiogroup" aria-label="Board theme">
                    {shared::BOARD_THEMES.iter().map(|(id, _)| {
                        let id = *id;
                        view! {
                            <BoardSwatch
                                theme=id
                                selected=Signal::derive(move || current_board() == id)
                                on_pick=pick_board
                            />
                        }
                    }).collect_view()}
                </div>
            </div>

            <div class="flex flex-col gap-3 px-4 py-4">
                <div class="flex flex-col gap-0.5">
                    <span class="text-sm font-semibold text-white">"Pieces"</span>
                    <span class="text-xs text-zinc-500">
                        "Applies everywhere a board is drawn — games, analysis and puzzles."
                    </span>
                </div>
                <div class="flex flex-wrap gap-2" role="radiogroup" aria-label="Piece set">
                    {shared::PIECE_SETS.iter().map(|(id, label)| {
                        let (id, label) = (*id, *label);
                        view! {
                            <PieceSwatch
                                set=id
                                label=label
                                selected=Signal::derive(move || current_pieces() == id)
                                on_pick=pick_pieces
                            />
                        }
                    }).collect_view()}
                </div>
            </div>
        </div>
    }
}
