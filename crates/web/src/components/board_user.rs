use leptos::prelude::*;
use shared::PlayerInfo;

#[component]
pub fn BoardUser(player: PlayerInfo) -> impl IntoView {
    view! {
        <div class="flex flex-row items-center gap-2 py-2">
            {player.avatar_url.map(|url| view! {
                <img src={url} class="w-8 h-8 rounded-full" />
            })}
            <span class="font-medium">
                {player.username.unwrap_or_else(|| "Anonymous".to_string())}
            </span>
            <span class="text-zinc-400">{player.rating}</span>
        </div>
    }
}
