use convert_case::{Case, Casing};
use leptos::prelude::*;
use shared::Category;
use uuid::Uuid;

use crate::components::use_current_user;

#[server]
pub async fn get_user_rating_and_diff(
    id: Uuid,
    category: Category,
) -> Result<(u32, i32), ServerFnError> {
    use crate::state::AppState;

    let app_state = expect_context::<AppState>();
    app_state
        .rating_store
        .get_rating_with_diff(&id, category)
        .await
        .map_err(|e| ServerFnError::new(format!("Error getting rating for user {id}: {e}")))
}

fn category_icon(category: Category) -> AnyView {
    let color = match category {
        Category::Bullet => "text-orange-400/75",
        Category::Blitz => "text-amber-400/75",
        Category::Rapid => "text-sky-400/75",
        Category::Classical => "text-violet-400/75",
    };
    match category {
        Category::Bullet => view! {
            <svg class=color width="15" height="15" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z"/>
            </svg>
        }.into_any(),
        Category::Blitz => view! {
            <svg class=color width="15" height="15" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>
            </svg>
        }.into_any(),
        Category::Rapid => view! {
            <svg class=color width="15" height="15" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="10"/>
                <polyline points="12 6 12 12 16 14"/>
            </svg>
        }.into_any(),
        Category::Classical => view! {
            <svg class=color width="15" height="15" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <path d="M5 22h14M5 2h14M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22M7 2v4.172a2 2 0 0 1 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/>
            </svg>
        }.into_any(),
    }
}

#[component]
pub fn EloCard(category: Category) -> impl IntoView {
    let user = use_current_user();

    let user_rating = Resource::new(
        move || user.get(),
        move |u| async move {
            match u {
                Some(Ok(Some(user))) => get_user_rating_and_diff(user.id, category).await.ok(),
                _ => {
                    leptos::logging::warn!("unable to get user info!");
                    None
                }
            }
        },
    );

    let fallback = move || {
        view! {
            <div class="flex flex-col gap-3 p-5 rounded-2xl bg-zinc-900 border border-zinc-800 min-w-[128px]">
                <div class="w-4 h-4 rounded skeleton-shimmer"/>
                <div class="flex flex-col gap-2">
                    <div class="w-16 h-7 rounded skeleton-shimmer"/>
                    <div class="w-8 h-2.5 rounded skeleton-shimmer"/>
                    <div class="w-10 h-2 rounded skeleton-shimmer"/>
                </div>
            </div>
        }
    };

    view! {
        <Transition fallback=fallback>
            <div class="flex flex-col gap-3 p-5 rounded-2xl bg-zinc-900 border border-zinc-800
                        hover:border-zinc-700 hover:-translate-y-px active:translate-y-0
                        transition-all duration-200 min-w-[128px] cursor-default
                        shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]">
                {category_icon(category)}
                <div class="flex flex-col gap-1">
                    <span class="text-3xl font-bold tracking-tighter text-white leading-none">
                        {move || user_rating.get()
                            .flatten()
                            .map(|(r, _)| r.to_string())
                            .unwrap_or_else(|| "\u{2014}".to_string())}
                    </span>
                    <span class=move || {
                        let diff = user_rating.get().flatten().map(|(_, d)| d).unwrap_or(0);
                        if diff > 0 { "text-[11px] font-semibold text-emerald-400/80" }
                        else if diff < 0 { "text-[11px] font-semibold text-red-400/80" }
                        else { "text-[11px] font-semibold text-zinc-600" }
                    }>
                        {move || match user_rating.get().flatten().map(|(_, d)| d) {
                            Some(d) if d > 0 => format!("+{d}"),
                            Some(d) if d < 0 => format!("{d}"),
                            _ => "\u{2014}".to_string(),
                        }}
                    </span>
                    <span class="text-[10px] font-semibold uppercase tracking-[0.12em] text-zinc-500">
                        {category.to_string().to_case(Case::Title)}
                    </span>
                </div>
            </div>
        </Transition>
    }
}
