use leptos::prelude::*;
use shared::{RecentGame, RecentGamePlayer, RecentGameResult, Side};

use crate::components::use_current_user;

#[server]
pub async fn get_recent_games() -> Result<Vec<RecentGame>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
    let Some(user_id) = auth.user.as_ref().map(|u| u.id) else {
        return Err(ServerFnError::ServerError("not signed in".to_string()));
    };
    let state = expect_context::<AppState>();
    Ok(state.game_store.list_recent_games(user_id, 10).await?)
}

fn player_name(p: &RecentGamePlayer) -> String {
    p.username.clone().unwrap_or_else(|| "Anonymous".to_string())
}

fn rating_text(rating: Option<i32>) -> String {
    rating.map(|r| r.to_string()).unwrap_or_else(|| "\u{2014}".to_string())
}

fn result_badge(result: RecentGameResult) -> (&'static str, &'static str) {
    match result {
        RecentGameResult::Won => ("\u{2713} Won", "text-emerald-400/80"),
        RecentGameResult::Lost => ("\u{2715} Lost", "text-red-400/80"),
        RecentGameResult::Drawn => ("\u{00bd} Draw", "text-zinc-300"),
        RecentGameResult::Aborted => ("\u{2014} Aborted", "text-zinc-500"),
    }
}

#[component]
fn PlayerHalf(player: RecentGamePlayer, is_me: bool) -> impl IntoView {
    view! {
        <div class="flex flex-row items-center gap-2 min-w-0">
            {player.avatar_url.clone().map(|url| view! {
                <img src={url} class="w-6 h-6 rounded-full flex-shrink-0" />
            })}
            <span
                class="truncate min-w-0"
                class:font-semibold=is_me
                class:text-white=is_me
                class:text-zinc-300=move || !is_me
            >
                {player_name(&player)}
            </span>
            <span class="text-zinc-500 text-sm flex-shrink-0">{rating_text(player.rating)}</span>
        </div>
    }
}

#[component]
fn RecentGameRow(game: RecentGame) -> impl IntoView {
    let (label, color_class) = result_badge(game.my_result);
    let href = format!("/analysis?game={}", game.id);

    view! {
        <a
            href={href}
            class="flex flex-row items-center justify-between gap-3 px-4 py-3 hover:bg-zinc-800/50 transition-colors"
        >
            <div class="flex flex-col gap-1 min-w-0">
                <PlayerHalf player={game.white.clone()} is_me={game.my_side == Side::White} />
                <PlayerHalf player={game.black.clone()} is_me={game.my_side == Side::Black} />
            </div>
            <span class={format!("text-sm font-semibold flex-shrink-0 {color_class}")}>{label}</span>
        </a>
    }
}

#[component]
pub fn RecentGames() -> impl IntoView {
    let user = use_current_user();

    let games = Resource::new(
        move || user.get(),
        move |u| async move {
            match u {
                Some(Ok(Some(_))) => get_recent_games().await.ok(),
                _ => None,
            }
        },
    );

    let fallback = move || {
        view! {
            <div class="flex flex-col gap-2">
                {(0..3).map(|_| view! {
                    <div class="h-14 rounded-xl skeleton-shimmer"/>
                }).collect_view()}
            </div>
        }
    };

    view! {
        <div class="px-6 pb-10">
            <h2 class="text-xl font-bold tracking-tighter text-white mb-4">"Recent games"</h2>
            <Transition fallback=fallback>
                {move || {
                    let rows = games.get().flatten().unwrap_or_default();
                    if rows.is_empty() {
                        view! {
                            <p class="text-zinc-500 text-sm italic px-1">"No games yet"</p>
                        }.into_any()
                    } else {
                        view! {
                            <div class="flex flex-col divide-y divide-zinc-800 rounded-2xl bg-zinc-900 border border-zinc-800 overflow-hidden">
                                {rows.into_iter().map(|game| view! { <RecentGameRow game={game} /> }).collect_view()}
                            </div>
                        }.into_any()
                    }
                }}
            </Transition>
        </div>
    }
}
