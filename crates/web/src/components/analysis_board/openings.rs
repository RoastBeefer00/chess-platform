//! Opening names for the position being viewed on the analysis board —
//! backed by the real lichess opening database (see `openings_data/`), not
//! a hand-picked subset. Parsed lazily, once, on first lookup.

use std::collections::HashMap;
use std::sync::OnceLock;

use shakmaty::{fen::Fen, Chess, EnPassantMode};

use super::tree::{MoveTree, NodeId};

const VOLUMES: [&str; 5] = [
    include_str!("openings_data/a.tsv"),
    include_str!("openings_data/b.tsv"),
    include_str!("openings_data/c.tsv"),
    include_str!("openings_data/d.tsv"),
    include_str!("openings_data/e.tsv"),
];

/// Board + side to move + castling rights + en-passant square, dropping
/// the halfmove/fullmove counters — two positions reached by different
/// paths (transpositions, or just replaying vs. loading a PGN) still key
/// the same. Both the catalogue (built by replaying each row's PGN below)
/// and every lookup go through this same function, so there's no risk of
/// the two sides disagreeing on how en passant is represented (unlike
/// comparing against an arbitrary externally-sourced FEN string).
fn position_key(pos: &Chess) -> String {
    let fen = Fen::from_position(pos, EnPassantMode::Legal).to_string();
    fen.split_whitespace().take(4).collect::<Vec<_>>().join(" ")
}

/// Parsed once, lazily: each TSV row's PGN movetext is replayed with the
/// existing `MoveTree::from_pgn` parser (the same one "Load" uses) to get
/// the position it reaches, keyed by `position_key`. A row whose movetext
/// fails to parse is skipped rather than panicking the whole catalogue —
/// defensive against any one bad line in externally-sourced data, not
/// expected to actually happen.
fn catalog() -> &'static HashMap<String, (String, String)> {
    static CATALOG: OnceLock<HashMap<String, (String, String)>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut map = HashMap::new();
        for volume in VOLUMES {
            for line in volume.lines().skip(1) {
                let mut fields = line.splitn(3, '\t');
                let (Some(eco), Some(name), Some(pgn)) = (fields.next(), fields.next(), fields.next()) else {
                    continue;
                };
                let Ok(tree) = MoveTree::from_pgn(pgn) else { continue };
                let end = tree.last_mainline_from(tree.root());
                let key = position_key(tree.position(end));
                map.entry(key).or_insert_with(|| (eco.to_string(), name.to_string()));
            }
        }
        map
    })
}

/// The name for the deepest position from the root to `cursor` (inclusive)
/// that's in the catalogue — walked from `cursor` back toward the root, so
/// a line that has gone past known theory still reports the most specific
/// named position it passed through, not nothing. `None` only once no
/// position along the way — including the current one — has a name (e.g.
/// still at the start position, or genuinely off the beaten path from
/// move one).
pub fn lookup(tree: &MoveTree, cursor: NodeId) -> Option<(String, String)> {
    let cat = catalog();
    tree.path_from_root(cursor)
        .into_iter()
        .rev()
        .find_map(|id| cat.get(&position_key(tree.position(id))).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_loads_the_full_dataset() {
        // Sanity check against a systematic parse regression — the real
        // dataset is ~3,800 rows; if a bug made most of them fail to
        // parse, this would catch it without pinning an exact count that
        // would need updating every time the vendored data does.
        assert!(catalog().len() > 3000, "only {} entries loaded", catalog().len());
    }

    fn play(moves: &[&str]) -> (MoveTree, NodeId) {
        let mut tree = MoveTree::new(Chess::default());
        let mut cursor = tree.root();
        for san in moves {
            let pos = tree.position(cursor).clone();
            let mv = san.parse::<shakmaty::san::San>().unwrap().to_move(&pos).unwrap();
            cursor = tree.play(cursor, mv, None);
        }
        (tree, cursor)
    }

    #[test]
    fn finds_the_vienna_gambit() {
        let (tree, cursor) = play(&["e4", "e5", "Nc3", "Nf6", "f4"]);
        let (eco, name) = lookup(&tree, cursor).expect("should find a name");
        // C29, not the more general C25 "Vienna Game" — the real database
        // is more precise than a guess at the code would have been.
        assert_eq!(eco, "C29");
        assert!(name.contains("Vienna"), "name was {name:?}");
    }

    #[test]
    fn narrows_from_sicilian_to_najdorf() {
        let (tree, cursor) = play(&["e4", "c5"]);
        let (_, name) = lookup(&tree, cursor).unwrap();
        assert!(name.contains("Sicilian"), "name was {name:?}");

        let (tree, cursor) = play(&["e4", "c5", "Nf3", "d6", "d4", "cxd4", "Nxd4", "Nf6", "Nc3", "a6"]);
        let (eco, name) = lookup(&tree, cursor).unwrap();
        assert_eq!(eco, "B90");
        assert!(name.contains("Najdorf"), "name was {name:?}");
    }

    #[test]
    fn falls_back_to_the_nearest_named_ancestor() {
        // A real but obscure continuation, unlikely to be its own row —
        // should still report back to whatever it transposed from/through.
        let (tree, cursor) = play(&["e4", "c5", "Nf3", "d6", "d4", "cxd4", "Nxd4", "Nf6", "Nc3", "a6", "h3"]);
        let (_, name) = lookup(&tree, cursor).unwrap();
        assert!(name.contains("Sicilian") || name.contains("Najdorf"), "name was {name:?}");
    }

    #[test]
    fn no_name_at_the_start_position() {
        let tree = MoveTree::new(Chess::default());
        assert_eq!(lookup(&tree, tree.root()), None);
    }
}
