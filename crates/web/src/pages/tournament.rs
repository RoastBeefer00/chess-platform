use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::{hooks::use_params_map, lazy_route, LazyRoute};
use uuid::Uuid;

use crate::components::{server_message, TournamentBody};
use crate::tournament::get_tournament;

#[derive(Clone)]
pub struct TournamentPage;

#[lazy_route]
impl LazyRoute for TournamentPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let params = use_params_map();
        let tournament_id = move || {
            params
                .read()
                .get("tournament_id")
                .and_then(|id| Uuid::parse_str(&id).ok())
        };

        // Loads the page once. Everything after that — the polling, the
        // organizer's actions — happens inside `TournamentBody`, which owns
        // the state from here on; see its doc comment for why this resource
        // is never refetched.
        let initial = Resource::new(tournament_id, |id| async move {
            match id {
                Some(id) => get_tournament(id).await,
                None => Err(ServerFnError::new("tournament not found")),
            }
        });

        let fallback = || {
            view! {
                <div class="px-6 py-8 max-w-5xl mx-auto flex flex-col gap-4">
                    <div class="h-10 w-2/3 rounded-control skeleton-shimmer"/>
                    <div class="h-64 rounded-card skeleton-shimmer"/>
                </div>
            }
        };

        view! {
            <Title text="Tournament"/>
            <Transition fallback=fallback>
                {move || initial.get().map(|result| match result {
                    Ok(view) => view! { <TournamentBody initial=view/> }.into_any(),
                    Err(e) => view! {
                        <div class="px-6 py-16 max-w-2xl mx-auto flex flex-col items-center gap-4 text-center">
                            <h1 class="display-1 text-white">"Tournament not found"</h1>
                            <p class="text-sm text-zinc-500">{server_message(&e)}</p>
                            <a href="/tournaments" class="btn-primary px-4 py-2 text-sm font-semibold">
                                "All tournaments"
                            </a>
                        </div>
                    }.into_any(),
                })}
            </Transition>
        }
        .into_any()
    }
}
