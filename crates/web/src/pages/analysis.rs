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
            let moves_param = q.get("moves")?;
            Some(PuzzleLoad {
                fen: q.get("fen")?.replace('_', " "),
                // An empty (but present) `moves=` shares the root position
                // itself (zero moves played) — `"".split('_')` would
                // otherwise yield `[""]`, one bogus empty-string "move".
                moves: if moves_param.is_empty() {
                    Vec::new()
                } else {
                    moves_param.split('_').map(String::from).collect()
                },
                ply: q.get("ply").and_then(|p| p.parse().ok()).unwrap_or(0),
            })
        };

        view! {
            <AnalysisBoard game_id={Signal::derive(game_id)} puzzle={Signal::derive(puzzle_load)}/>
        }
        .into_any()
    }
}
