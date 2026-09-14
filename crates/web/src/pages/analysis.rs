use leptos::prelude::*;
use leptos_router::{hooks::use_query_map, lazy_route, LazyRoute};
use uuid::Uuid;

use crate::components::{AnalysisBoard, PuzzleLoad};

#[derive(Clone)]
pub struct AnalysisPage;

#[lazy_route]
impl LazyRoute for AnalysisPage {
    fn data() -> Self {
        Self
    }

    fn view(_data: Self) -> AnyView {
        let query = use_query_map();
        let game_id =
            move || query.read().get("game").and_then(|id| Uuid::parse_str(&id).ok());
        let puzzle_load = move || {
            let q = query.read();
            Some(PuzzleLoad {
                fen: q.get("fen")?.replace('_', " "),
                moves: q.get("moves")?.split('_').map(String::from).collect(),
                ply: q.get("ply").and_then(|p| p.parse().ok()).unwrap_or(0),
            })
        };

        view! {
            <AnalysisBoard game_id={Signal::derive(game_id)} puzzle={Signal::derive(puzzle_load)}/>
        }
        .into_any()
    }
}
