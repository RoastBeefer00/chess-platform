use leptos::prelude::*;
use shared::{ActiveGame, RecentGamePlayer};

use crate::components::use_current_user;

#[server]
pub async fn get_active_game() -> Result<Option<ActiveGame>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let Some(user_id) = auth.user.as_ref().map(|u| u.id) else {
        return Err(ServerFnError::ServerError("not signed in".to_string()));
    };
    let state = expect_context::<AppState>();
    Ok(state.game_store.find_active_game(user_id).await?)
}

/// The signed-in user's in-progress game, fetched once per page load.
/// Root-owned for the same reason as [`crate::components::MyRatingsResource`].
///
/// Still keyed on the current-user resource so it issues no query at all
/// until there is a user to query for.
#[derive(Copy, Clone)]
pub struct MyActiveGameResource(pub Resource<Option<ActiveGame>>);

/// Call once at the App root.
pub fn provide_my_active_game() {
    let user = use_current_user();
    provide_context(MyActiveGameResource(Resource::new(
        move || user.get(),
        move |u| async move {
            match u {
                Some(Ok(Some(_))) => get_active_game().await.ok().flatten(),
                _ => None,
            }
        },
    )));
}

pub fn use_my_active_game() -> Resource<Option<ActiveGame>> {
    use_context::<MyActiveGameResource>()
        .expect("provide_my_active_game must be called at the App root")
        .0
}

fn opponent_name(p: &RecentGamePlayer) -> String {
    p.username.clone().unwrap_or_else(|| "Anonymous".to_string())
}

/// Banner offering to jump back into a game the user is still in — shown
/// after a dropped tab, accidental navigation, or a switch to another
/// device, since the game itself lives server-side, not in the tab.
#[component]
pub fn ResumeGame() -> impl IntoView {
    let active_game = use_my_active_game();

    view! {
        <Transition fallback=|| ()>
            {move || {
                active_game.get().flatten().map(|game| {
                    let href = format!("/game/{}", game.id);
                    view! {
                        <a
                            href={href}
                            class="mx-6 mt-6 flex items-center justify-between gap-3 px-4 py-3
                                   rounded-card bg-emerald-500/10 border border-emerald-500/30
                                   hover:bg-emerald-500/15 transition-colors"
                        >
                            <div class="flex items-center gap-3 min-w-0">
                                {game.opponent.avatar_url.clone().map(|url| view! {
                                    <img src={url} alt="" class="w-8 h-8 rounded-full flex-shrink-0" />
                                })}
                                <div class="flex flex-col min-w-0">
                                    <span class="text-sm font-semibold text-white">
                                        "Game in progress"
                                    </span>
                                    <span class="text-xs text-zinc-400 truncate">
                                        "vs " {opponent_name(&game.opponent)}
                                    </span>
                                </div>
                            </div>
                            <span class="text-xs font-semibold uppercase tracking-wide text-emerald-400 flex-shrink-0">
                                "Resume \u{2192}"
                            </span>
                        </a>
                    }
                })
            }}
        </Transition>
    }
}
