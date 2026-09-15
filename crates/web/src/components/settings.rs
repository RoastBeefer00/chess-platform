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
    Ok(state.user_store.update_settings(user_id, &settings).await?)
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
#[component]
pub fn Settings() -> impl IntoView {
    let user = use_current_user();
    let auth_trigger = use_auth_trigger();

    let server_auto_queen = move || {
        user.get()
            .and_then(|r| r.ok())
            .flatten()
            .map(|u| u.settings.auto_queen)
            .unwrap_or(false)
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
        let settings = *settings;
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

    let toggle_auto_queen = move |_: ()| {
        let new_value = !auto_queen();
        auto_queen_override.set(Some(new_value));
        save.dispatch(UserSettings {
            auto_queen: new_value,
        });
    };

    view! {
        <div class="flex flex-col divide-y divide-zinc-800 rounded-2xl bg-zinc-900 border border-zinc-800 overflow-hidden">
            <div class="flex items-center justify-between gap-4 px-4 py-4">
                <div class="flex flex-col gap-0.5 min-w-0">
                    <span class="text-sm font-semibold text-white">"Auto-queen"</span>
                    <span class="text-xs text-zinc-500">
                        "Always promote pawns to a queen, skipping the picker."
                    </span>
                </div>
                <Toggle checked=Signal::derive(auto_queen) on_toggle=Callback::new(toggle_auto_queen) />
            </div>
        </div>
    }
}
