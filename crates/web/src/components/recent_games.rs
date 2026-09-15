use leptos::prelude::*;
use shared::{RecentGame, RecentGamePlayer, RecentGameResult, Side};

/// `username: None` means "the signed-in caller" — the shape existing call
/// sites (the play hub, showing your own recent games) already rely on.
/// Passing a specific username (the profile page, showing anyone's) requires
/// no special privilege — recent game results are public the same way they
/// already are via `/analysis?game=`.
///
/// Takes a username rather than a `Uuid` so `RecentGames` needs no
/// dependency on any other resource resolving first — nesting a fresh
/// `Resource` inside another resource's already-resolved branch (as the
/// profile page originally did, keying off a separately-fetched user id)
/// intermittently desynced SSR and hydration for sibling resources created
/// that way.
#[server]
pub async fn get_recent_games(username: Option<String>) -> Result<Vec<RecentGame>, ServerFnError> {
    use crate::auth::AuthBackend;
    use crate::state::AppState;
    use axum_login::AuthSession;

    let state = expect_context::<AppState>();
    let user_id = match username {
        Some(name) => state
            .user_store
            .find_by_username(&name)
            .await?
            .ok_or_else(|| ServerFnError::new("user not found"))?
            .id,
        None => {
            let auth = leptos_axum::extract::<AuthSession<AuthBackend>>().await?;
            auth.user.as_ref().map(|u| u.id).ok_or_else(|| ServerFnError::new("not signed in"))?
        }
    };
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
            <div class="flex flex-col items-end gap-1 flex-shrink-0">
                <span class={format!("text-sm font-semibold {color_class}")}>{label}</span>
                <span class="text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-500">
                    {if game.rated { "Rated" } else { "Casual" }}
                </span>
            </div>
        </a>
    }
}

#[component]
pub fn RecentGames(#[prop(optional)] username: Option<String>) -> impl IntoView {
    let games = Resource::new(
        move || username.clone(),
        move |username| async move { get_recent_games(username).await.ok() },
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
        <div>
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
