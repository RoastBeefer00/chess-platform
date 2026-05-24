use leptos::prelude::*;
use shared::PlayerInfo;

#[component]
pub fn BoardUser(player: PlayerInfo) -> impl IntoView {
    view! {
        <div class="flex flex-row items-center gap-2 py-2 min-w-0">
            {player.avatar_url.map(|url| view! {
                <img src={url} class="w-8 h-8 rounded-full flex-shrink-0" />
            })}
            <span class="font-medium truncate min-w-0">
                {player.username.unwrap_or_else(|| "Anonymous".to_string())}
            </span>
            <span class="text-zinc-400 flex-shrink-0">{player.rating}</span>
        </div>
    }
}
