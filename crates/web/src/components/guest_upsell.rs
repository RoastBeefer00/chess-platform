use leptos::prelude::*;

use crate::components::auth::use_current_user;

/// A nudge to sign in, shown only to guest accounts.
///
/// Guests are one click to create and permanent rows in `users`, but they're
/// locked out of rated play and the whole friends feature, and nothing ever
/// told them so at a moment they'd care. This renders at the end of a game —
/// the point where someone has just decided they like the site.
///
/// The copy is deliberately forward-looking ("play rated", "add friends")
/// rather than promising their current history carries over: signing in
/// creates a *new* account via OAuth, and there is no guest-to-real account
/// linking, so a guest's existing games do not follow them. Saying otherwise
/// would be a lie the next screen immediately exposes.
#[component]
pub fn GuestUpsell(
    /// Extra classes for the wrapper, so a caller can match its surroundings.
    #[prop(optional, default = "")]
    class: &'static str,
) -> impl IntoView {
    let user = use_current_user();
    let is_guest = move || {
        user.get()
            .and_then(|r| r.ok())
            .flatten()
            .is_some_and(|u| u.is_guest)
    };

    view! {
        <Show when=is_guest>
            <div class=format!(
                "flex flex-col gap-2 rounded-control border border-zinc-800 bg-zinc-950/60 p-3 text-center {class}"
            )>
                <span class="text-xs text-zinc-400">
                    "You're playing as a guest. Sign in to play rated games, add friends, and keep your history."
                </span>
                <a
                    href="/login"
                    rel="external"
                    class="px-4 py-2 text-sm font-semibold bg-white text-zinc-950 rounded-control hover:bg-zinc-100 transition-colors"
                >
                    "Sign in"
                </a>
            </div>
        </Show>
    }
}
