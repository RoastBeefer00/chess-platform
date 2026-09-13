//! Move tree for the analysis board: a game tree of positions with support
//! for variations, branching off any node, and (optionally) a clock snapshot
//! per node. Pure data — no Leptos — so it's plain to unit test.

use shakmaty::{
    san::{San, SanPlus},
    CastlingMode, Chess, Color, Move, Position,
};

pub type NodeId = usize;

#[derive(Debug, Clone)]
pub struct MoveNode {
    pub parent: Option<NodeId>,
    /// `children[0]` is the mainline continuation; anything after that is a
    /// variation.
    pub children: Vec<NodeId>,
    /// UCI of the move that reached this node. Empty for the root.
    pub uci: String,
    /// SAN of the move that reached this node, rendered against the parent
    /// position. Empty for the root.
    pub san: String,
    /// Position after this node's move (or the root position, for node 0).
    pub position: Chess,
    /// (white_ms_left, black_ms_left) as of this move, when known.
    pub clock: Option<(i64, i64)>,
}

/// A contiguous run of moves at one depth, as produced by [`MoveTree::flatten`].
/// Depth 0 is the mainline; each nesting level of `( )` in a PGN adds one.
#[derive(Debug, Clone)]
pub struct Segment {
    pub depth: usize,
    pub nodes: Vec<DisplayNode>,
}

/// One continuous line — the mainline, or a single variation — as a
/// sequence of its own move-runs interleaved with any variations that
/// branch off it. Unlike [`Segment`]/[`MoveTree::flatten`]'s flat list, this
/// preserves which move-runs belong to the *same* line (so they can render
/// as one continuous flow) versus which are genuinely separate sibling
/// lines (so each gets its own `( )`) — a distinction depth alone can't
/// recover once two sibling variations share a branch point, since both
/// share the same depth and sit consecutively in the flat form.
#[derive(Debug, Clone)]
pub struct MoveLine {
    pub depth: usize,
    pub items: Vec<LineItem>,
}

#[derive(Debug, Clone)]
pub enum LineItem {
    /// A contiguous run of this line's own moves, before it either ends or
    /// is interrupted by a variation branching off it.
    Moves(Vec<DisplayNode>),
    /// A variation branching off at this point — a fully separate line, one
    /// depth deeper.
    Variation(MoveLine),
}

#[derive(Debug, Clone)]
pub struct DisplayNode {
    pub id: NodeId,
    pub san: String,
    /// PGN move number (the parent position's fullmove counter).
    pub fullmove: u32,
    pub is_white: bool,
    /// Whether the move number should be rendered before this move — every
    /// white move, plus the first move of any segment (so a variation
    /// starting on black renders as `12...Qd7`).
    pub show_number: bool,
}

#[derive(Debug, Clone)]
pub struct MoveTree {
    nodes: Vec<MoveNode>,
}

impl MoveTree {
    /// New tree rooted at `root`, with no clock data.
    pub fn new(root: Chess) -> Self {
        Self {
            nodes: vec![MoveNode {
                parent: None,
                children: Vec::new(),
                uci: String::new(),
                san: String::new(),
                position: root,
                clock: None,
            }],
        }
    }

    /// New tree rooted at `root`, seeded with `initial_ms` on the clock for
    /// both sides (e.g. when starting a fresh analysis of a timed game).
    pub fn with_clocks(root: Chess, initial_ms: i64) -> Self {
        let mut tree = Self::new(root);
        tree.nodes[0].clock = Some((initial_ms, initial_ms));
        tree
    }

    pub fn root(&self) -> NodeId {
        0
    }

    pub fn position(&self, id: NodeId) -> &Chess {
        &self.nodes[id].position
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id].parent
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id].children
    }

    pub fn san(&self, id: NodeId) -> &str {
        &self.nodes[id].san
    }

    pub fn uci(&self, id: NodeId) -> &str {
        &self.nodes[id].uci
    }

    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id].children.first().copied()
    }

    /// Follows `children[0]` from `id` to the end of its mainline.
    pub fn last_mainline_from(&self, id: NodeId) -> NodeId {
        let mut cur = id;
        while let Some(next) = self.first_child(cur) {
            cur = next;
        }
        cur
    }

    /// Plays `m` from `at`. If a child with the same UCI already exists,
    /// returns it (transposing back into an existing line) instead of
    /// duplicating it; otherwise appends a new child.
    pub fn play(&mut self, at: NodeId, m: Move, clock: Option<(i64, i64)>) -> NodeId {
        let uci = m.to_uci(CastlingMode::Standard).to_string();
        if let Some(&existing) = self.nodes[at].children.iter().find(|&&c| self.nodes[c].uci == uci) {
            return existing;
        }
        let mut new_pos = self.nodes[at].position.clone();
        let san_plus = SanPlus::from_move_and_play_unchecked(&mut new_pos, m);
        let id = self.nodes.len();
        self.nodes.push(MoveNode {
            parent: Some(at),
            children: Vec::new(),
            uci,
            san: san_plus.to_string(),
            position: new_pos,
            clock,
        });
        self.nodes[at].children.push(id);
        id
    }

    /// Unlinks `id` (and everything under it) from its parent. The root
    /// (id 0) can't be deleted. Orphaned nodes stay in the arena — dead
    /// weight, not worth a full rebuild for.
    pub fn delete(&mut self, id: NodeId) {
        if id == 0 {
            return;
        }
        if let Some(p) = self.nodes[id].parent {
            self.nodes[p].children.retain(|&c| c != id);
        }
    }

    /// Moves `id` to the front of its parent's children, making it the new
    /// mainline continuation.
    pub fn promote(&mut self, id: NodeId) {
        if let Some(p) = self.nodes[id].parent {
            let siblings = &mut self.nodes[p].children;
            if let Some(pos) = siblings.iter().position(|&c| c == id) {
                siblings.remove(pos);
                siblings.insert(0, id);
            }
        }
    }

    pub fn path_from_root(&self, id: NodeId) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            path.push(c);
            cur = self.nodes[c].parent;
        }
        path.reverse();
        path
    }

    /// Clock at `id`, inherited from the nearest ancestor that has one. A
    /// move played in a variation (which never had its own clock recorded)
    /// reads as whatever the clocks were at the branch point.
    pub fn clock_at(&self, id: NodeId) -> Option<(i64, i64)> {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if let Some(clock) = self.nodes[c].clock {
                return Some(clock);
            }
            cur = self.nodes[c].parent;
        }
        None
    }

    /// Records that after `id`'s move, the mover had `ms` remaining. The
    /// other side's clock is carried over from the nearest ancestor (its
    /// clock didn't run during this move). No-op on the root, which has no
    /// "mover". Used when importing a PGN's `%clk` comments, which follow
    /// the move they describe rather than preceding it.
    fn set_mover_clock(&mut self, id: NodeId, ms: i64) {
        let Some(parent) = self.nodes[id].parent else {
            return;
        };
        let (parent_w, parent_b) = self.clock_at(parent).unwrap_or((0, 0));
        let clock = match self.nodes[parent].position.turn() {
            Color::White => (ms, parent_b),
            Color::Black => (parent_w, ms),
        };
        self.nodes[id].clock = Some(clock);
    }

    /// Flattens the tree into contiguous line segments for rendering: the
    /// mainline runs as depth-0 segments, and each `( )` nesting level in a
    /// PGN becomes one more depth. A branch flushes the segment in progress,
    /// recurses into every non-mainline child as its own (deeper) segment,
    /// then resumes the mainline as a fresh segment at the original depth.
    pub fn flatten(&self) -> Vec<Segment> {
        let mut out = Vec::new();
        for (i, &child) in self.nodes[0].children.iter().enumerate() {
            // Root-level alternates (more than one possible first move) have
            // no preceding move to flush a segment for, so they're rendered
            // as fully separate lines rather than interleaved mid-walk like
            // nested variations are. Rare in practice.
            self.flatten_line(child, if i == 0 { 0 } else { 1 }, &mut out);
        }
        out
    }

    fn flatten_line(&self, first_move: NodeId, depth: usize, out: &mut Vec<Segment>) {
        let mut nodes = Vec::new();
        let mut cur = first_move;
        loop {
            let node = &self.nodes[cur];
            let parent_pos = &self.nodes[node.parent.expect("flatten_line never visits the root")].position;
            let is_white = parent_pos.turn() == Color::White;
            nodes.push(DisplayNode {
                id: cur,
                san: node.san.clone(),
                fullmove: parent_pos.fullmoves().get(),
                is_white,
                show_number: nodes.is_empty() || is_white,
            });

            match node.children.as_slice() {
                [] => break,
                [only] => cur = *only,
                [mainline, rest @ ..] => {
                    out.push(Segment { depth, nodes: std::mem::take(&mut nodes) });
                    for &variation in rest {
                        self.flatten_line(variation, depth + 1, out);
                    }
                    cur = *mainline;
                }
            }
        }
        if !nodes.is_empty() {
            out.push(Segment { depth, nodes });
        }
    }

    /// Same traversal as [`Self::flatten`], but preserving line structure
    /// (see [`MoveLine`]) instead of flattening it away — what the analysis
    /// board's move panel renders from, so the mainline (and each
    /// variation's own continuation around any sub-variations) can flow as
    /// one continuous set of moves, with each variation breaking in as its
    /// own indented block exactly where it branches off.
    pub fn flatten_tree(&self) -> MoveLine {
        let mut items = Vec::new();
        for (i, &child) in self.nodes[0].children.iter().enumerate() {
            if i == 0 {
                items.extend(self.line_items(child, 0));
            } else {
                // Root-level alternates — see the comment in `flatten`.
                items.push(LineItem::Variation(self.build_line(child, 1)));
            }
        }
        MoveLine { depth: 0, items }
    }

    fn build_line(&self, first_move: NodeId, depth: usize) -> MoveLine {
        MoveLine { depth, items: self.line_items(first_move, depth) }
    }

    fn line_items(&self, first_move: NodeId, depth: usize) -> Vec<LineItem> {
        let mut items = Vec::new();
        let mut nodes = Vec::new();
        let mut cur = first_move;
        loop {
            let node = &self.nodes[cur];
            let parent_pos = &self.nodes[node.parent.expect("line_items never visits the root")].position;
            let is_white = parent_pos.turn() == Color::White;
            nodes.push(DisplayNode {
                id: cur,
                san: node.san.clone(),
                fullmove: parent_pos.fullmoves().get(),
                is_white,
                show_number: nodes.is_empty() || is_white,
            });

            match node.children.as_slice() {
                [] => break,
                [only] => cur = *only,
                [mainline, rest @ ..] => {
                    items.push(LineItem::Moves(std::mem::take(&mut nodes)));
                    for &variation in rest {
                        items.push(LineItem::Variation(self.build_line(variation, depth + 1)));
                    }
                    cur = *mainline;
                }
            }
        }
        if !nodes.is_empty() {
            items.push(LineItem::Moves(nodes));
        }
        items
    }

    /// Renders the tree as PGN movetext: mainline inline, every non-first
    /// child of a node wrapped in `( )`, with a `{[%clk H:MM:SS]}` comment
    /// after any move that carries a clock.
    pub fn to_pgn(&self) -> String {
        let mut out = String::new();
        for (i, &child) in self.nodes[0].children.iter().enumerate() {
            if i > 0 {
                out.push_str("( ");
            }
            self.emit_move(child, &mut out, true);
            if i > 0 {
                out.push_str(") ");
            }
        }
        out.trim_end().to_string()
    }

    /// Writes just `id`'s own move number (when `force_number`, or always
    /// for white), SAN, and clock comment — no continuation.
    fn write_move_text(&self, id: NodeId, out: &mut String, force_number: bool) {
        let node = &self.nodes[id];
        let parent_pos = &self.nodes[node.parent.expect("write_move_text never visits the root")].position;
        let is_white = parent_pos.turn() == Color::White;
        if is_white || force_number {
            let n = parent_pos.fullmoves().get();
            out.push_str(&format!("{n}{} ", if is_white { "." } else { "..." }));
        }
        out.push_str(&node.san);
        if let Some((w, b)) = node.clock {
            let ms = if is_white { w } else { b };
            out.push_str(&format!(" {{[%clk {}]}}", format_clock_timestamp(ms)));
        }
        out.push(' ');
    }

    /// Writes `id`'s own move text, then whatever follows it.
    fn emit_move(&self, id: NodeId, out: &mut String, force_number: bool) {
        self.write_move_text(id, out, force_number);
        self.emit_continuation(id, out, false);
    }

    /// Writes what comes after `id`'s own move text: nothing, a single
    /// continuation, or — for a branch — the mainline continuation's own
    /// text first, each other child as a `( )` variation right after it
    /// (standard PGN order: an alternative follows the move it replaces),
    /// then the rest of the mainline. `force_number` applies to whichever
    /// move is emitted next.
    fn emit_continuation(&self, id: NodeId, out: &mut String, force_number: bool) {
        match self.nodes[id].children.as_slice() {
            [] => {}
            [only] => self.emit_move(*only, out, force_number),
            [mainline, rest @ ..] => {
                let mainline = *mainline;
                let variations: Vec<NodeId> = rest.to_vec();
                self.write_move_text(mainline, out, force_number);
                for variation in variations {
                    out.push_str("( ");
                    self.emit_move(variation, out, true);
                    out.push_str(") ");
                }
                // A move number is restated after a variation closes, even
                // if the resumed move is black's.
                self.emit_continuation(mainline, out, true);
            }
        }
    }

    /// Parses PGN movetext (with or without `[Tag "..."]` headers) into a
    /// tree, honoring `( )` variations and `{[%clk ...]}` clock comments.
    /// Errors with a human-readable message on malformed input rather than
    /// panicking.
    pub fn from_pgn(pgn: &str) -> Result<MoveTree, String> {
        let mut tree = MoveTree::new(Chess::default());
        let mut cursor: NodeId = 0;
        let mut resume_stack: Vec<NodeId> = Vec::new();

        let bytes = pgn.as_bytes();
        let n = bytes.len();
        let mut i = 0;
        while i < n {
            let c = bytes[i] as char;
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            match c {
                ';' => {
                    while i < n && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                '[' => {
                    while i < n && bytes[i] != b']' {
                        i += 1;
                    }
                    i += 1;
                }
                '{' => {
                    let start = i + 1;
                    let mut j = start;
                    while j < n && bytes[j] != b'}' {
                        j += 1;
                    }
                    // A `%clk` comment follows the move it describes (per
                    // the PGN convention), so it applies to `cursor` — the
                    // node just played — not to whatever comes next.
                    if let Some(ms) = parse_clk_comment(&pgn[start..j.min(n)]) {
                        tree.set_mover_clock(cursor, ms);
                    }
                    i = j + 1;
                }
                '(' => {
                    resume_stack.push(cursor);
                    cursor = tree
                        .parent(cursor)
                        .ok_or("PGN has a variation '(' with no move to branch from")?;
                    i += 1;
                }
                ')' => {
                    cursor = resume_stack.pop().ok_or("PGN has an unmatched ')'")?;
                    i += 1;
                }
                '$' => {
                    while i < n && !(bytes[i] as char).is_whitespace() {
                        i += 1;
                    }
                }
                _ => {
                    let start = i;
                    while i < n {
                        let ch = bytes[i] as char;
                        if ch.is_whitespace() || "(){};[".contains(ch) {
                            break;
                        }
                        i += 1;
                    }
                    let token = &pgn[start..i];
                    if is_move_number_token(token) || is_result_token(token) {
                        continue;
                    }
                    let san: San = token
                        .parse()
                        .map_err(|_| format!("invalid move text: {token:?}"))?;
                    let pos = tree.position(cursor).clone();
                    let mv = san
                        .to_move(&pos)
                        .map_err(|_| format!("illegal move: {token:?}"))?;
                    cursor = tree.play(cursor, mv, None);
                }
            }
        }

        if !resume_stack.is_empty() {
            return Err("PGN has an unmatched '('".to_string());
        }
        Ok(tree)
    }

    /// Builds a linear (no variations) tree by replaying a stored game's UCI
    /// move list, e.g. loading a finished gambit.rs game into the analysis
    /// board. `clocks` is index-aligned with `moves`; a `None` entry plays
    /// the move without recording a clock on that node (it'll inherit the
    /// nearest ancestor's via `clock_at`, same as a variation move does).
    pub fn from_uci_moves(
        moves: &[String],
        clocks: &[Option<(i64, i64)>],
        initial_time_ms: i64,
    ) -> Result<MoveTree, String> {
        let mut tree = MoveTree::with_clocks(Chess::default(), initial_time_ms);
        let mut cursor = tree.root();
        for (i, uci_str) in moves.iter().enumerate() {
            let uci: shakmaty::uci::UciMove = uci_str
                .parse()
                .map_err(|_| format!("bad move: {uci_str:?}"))?;
            let pos = tree.position(cursor).clone();
            let mv = uci
                .to_move(&pos)
                .map_err(|_| format!("illegal move: {uci_str:?}"))?;
            let clock = clocks.get(i).copied().flatten();
            cursor = tree.play(cursor, mv, clock);
        }
        Ok(tree)
    }
}

fn is_move_number_token(token: &str) -> bool {
    let digits_end = token.find(|c: char| !c.is_ascii_digit()).unwrap_or(token.len());
    digits_end > 0 && token.contains('.') && token[digits_end..].chars().all(|c| c == '.')
}

fn is_result_token(token: &str) -> bool {
    matches!(token, "1-0" | "0-1" | "1/2-1/2" | "*")
}

/// Parses a `%clk` payload like `1:23:45` or `1:23:45.6` into milliseconds.
fn parse_clk_comment(comment: &str) -> Option<i64> {
    let idx = comment.find("%clk")?;
    let rest = comment[idx + 4..].trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.'))
        .unwrap_or(rest.len());
    parse_clock_timestamp(&rest[..end])
}

fn parse_clock_timestamp(ts: &str) -> Option<i64> {
    let mut parts = ts.split(':');
    let h: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(h * 3_600_000 + m * 60_000 + (s * 1000.0).round() as i64)
}

fn format_clock_timestamp(ms: i64) -> String {
    let ms = ms.max(0);
    let total_seconds = ms / 1000;
    let h = total_seconds / 3600;
    let m = (total_seconds % 3600) / 60;
    let s = total_seconds % 60;
    format!("{h}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::uci::UciMove;

    fn play_uci(tree: &mut MoveTree, at: NodeId, uci: &str) -> NodeId {
        let pos = tree.position(at).clone();
        let m = uci.parse::<UciMove>().unwrap().to_move(&pos).unwrap();
        tree.play(at, m, None)
    }

    #[test]
    fn playing_same_move_twice_reuses_the_node() {
        let mut tree = MoveTree::new(Chess::default());
        let a = play_uci(&mut tree, 0, "e2e4");
        let b = play_uci(&mut tree, 0, "e2e4");
        assert_eq!(a, b);
        assert_eq!(tree.children(0), &[a]);
    }

    #[test]
    fn different_move_from_rewound_node_creates_a_sibling() {
        let mut tree = MoveTree::new(Chess::default());
        let e4 = play_uci(&mut tree, 0, "e2e4");
        let d4 = play_uci(&mut tree, 0, "d2d4");
        assert_ne!(e4, d4);
        assert_eq!(tree.children(0), &[e4, d4]);
    }

    #[test]
    fn flatten_numbers_and_depth() {
        let mut tree = MoveTree::new(Chess::default());
        let e4 = play_uci(&mut tree, 0, "e2e4");
        let e5 = play_uci(&mut tree, e4, "e7e5");
        let nf3 = play_uci(&mut tree, e5, "g1f3");
        let d4 = play_uci(&mut tree, e5, "d2d4");
        play_uci(&mut tree, nf3, "b8c6");
        play_uci(&mut tree, d4, "e5d4");

        let segments = tree.flatten();
        assert_eq!(segments.len(), 3);

        assert_eq!(segments[0].depth, 0);
        let mainline_opening: Vec<&str> = segments[0].nodes.iter().map(|n| n.san.as_str()).collect();
        assert_eq!(mainline_opening, ["e4", "e5"]);
        assert_eq!(segments[0].nodes[0].fullmove, 1);
        assert!(segments[0].nodes[0].is_white);
        assert!(segments[0].nodes[0].show_number);
        assert!(!segments[0].nodes[1].show_number);

        assert_eq!(segments[1].depth, 1);
        let variation: Vec<&str> = segments[1].nodes.iter().map(|n| n.san.as_str()).collect();
        assert_eq!(variation, ["d4", "exd4"]);
        assert!(segments[1].nodes[0].show_number);

        assert_eq!(segments[2].depth, 0);
        let resumed: Vec<&str> = segments[2].nodes.iter().map(|n| n.san.as_str()).collect();
        assert_eq!(resumed, ["Nf3", "Nc6"]);
        assert_eq!(segments[2].nodes[0].fullmove, 2);
    }

    fn sans(nodes: &[DisplayNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.san.as_str()).collect()
    }

    #[test]
    fn flatten_tree_flows_mainline_around_a_single_variation() {
        let mut tree = MoveTree::new(Chess::default());
        let e4 = play_uci(&mut tree, 0, "e2e4");
        let e5 = play_uci(&mut tree, e4, "e7e5");
        let nf3 = play_uci(&mut tree, e5, "g1f3");
        let d4 = play_uci(&mut tree, e5, "d2d4");
        play_uci(&mut tree, nf3, "b8c6");
        play_uci(&mut tree, d4, "e5d4");

        let top = tree.flatten_tree();
        assert_eq!(top.depth, 0);
        // Mainline before the branch, the variation, then the mainline
        // continuation — the latter two must NOT collapse into one item,
        // but "e4 e5" and "Nf3 Nc6" are the same line and belong to the
        // same top-level `MoveLine`, ready to render as one flow around the
        // variation that interrupts it.
        assert_eq!(top.items.len(), 3);

        let LineItem::Moves(opening) = &top.items[0] else { panic!("expected Moves") };
        assert_eq!(sans(opening), ["e4", "e5"]);

        let LineItem::Variation(variation) = &top.items[1] else { panic!("expected Variation") };
        assert_eq!(variation.depth, 1);
        assert_eq!(variation.items.len(), 1);
        let LineItem::Moves(var_moves) = &variation.items[0] else { panic!("expected Moves") };
        assert_eq!(sans(var_moves), ["d4", "exd4"]);

        let LineItem::Moves(resumed) = &top.items[2] else { panic!("expected Moves") };
        assert_eq!(sans(resumed), ["Nf3", "Nc6"]);
    }

    #[test]
    fn flatten_tree_keeps_sibling_variations_separate() {
        // Two variations branching off the *same* point (both children of
        // `e4`, both depth 1) used to merge into a single shared `( )` when
        // the move panel reconstructed line structure from `flatten`'s flat,
        // depth-tagged segments alone — depth doesn't distinguish "two
        // consecutive segments belonging to one interrupted line" from "two
        // separate sibling lines that happen to share a depth". `flatten_tree`
        // preserves this directly from the tree instead of reconstructing it.
        let mut tree = MoveTree::new(Chess::default());
        let e4 = play_uci(&mut tree, 0, "e2e4");
        let e5 = play_uci(&mut tree, e4, "e7e5"); // mainline continuation
        play_uci(&mut tree, e4, "c7c5"); // variation 1
        play_uci(&mut tree, e4, "e7e6"); // variation 2
        play_uci(&mut tree, e5, "g1f3"); // mainline resumes

        let top = tree.flatten_tree();
        assert_eq!(top.items.len(), 4);

        let LineItem::Moves(opening) = &top.items[0] else { panic!("expected Moves") };
        assert_eq!(sans(opening), ["e4"]);

        let LineItem::Variation(v1) = &top.items[1] else { panic!("expected Variation") };
        assert_eq!(v1.depth, 1);
        let LineItem::Moves(v1_moves) = &v1.items[0] else { panic!("expected Moves") };
        assert_eq!(sans(v1_moves), ["c5"]);

        let LineItem::Variation(v2) = &top.items[2] else { panic!("expected Variation") };
        assert_eq!(v2.depth, 1);
        let LineItem::Moves(v2_moves) = &v2.items[0] else { panic!("expected Moves") };
        assert_eq!(sans(v2_moves), ["e6"]);

        let LineItem::Moves(resumed) = &top.items[3] else { panic!("expected Moves") };
        assert_eq!(sans(resumed), ["e5", "Nf3"]);
    }

    #[test]
    fn promote_reorders_children() {
        let mut tree = MoveTree::new(Chess::default());
        let e4 = play_uci(&mut tree, 0, "e2e4");
        let d4 = play_uci(&mut tree, 0, "d2d4");
        assert_eq!(tree.children(0), &[e4, d4]);
        tree.promote(d4);
        assert_eq!(tree.children(0), &[d4, e4]);
    }

    #[test]
    fn delete_unlinks_subtree_but_keeps_root() {
        let mut tree = MoveTree::new(Chess::default());
        let e4 = play_uci(&mut tree, 0, "e2e4");
        let d4 = play_uci(&mut tree, 0, "d2d4");
        tree.delete(d4);
        assert_eq!(tree.children(0), &[e4]);
        tree.delete(0);
        assert_eq!(tree.children(0), &[e4], "deleting the root is a no-op");
    }

    #[test]
    fn clock_at_inherits_from_branch_point() {
        let mut tree = MoveTree::with_clocks(Chess::default(), 180_000);
        let e4 = play_uci(&mut tree, 0, "e2e4"); // no explicit clock on e4
        let e5 = play_uci(&mut tree, e4, "e7e5");
        let d5_variation = play_uci(&mut tree, e4, "d7d5");
        // Neither e5 nor the d5 variation has its own recorded clock — both
        // should read the nearest ancestor's, i.e. the root's initial 180s.
        assert_eq!(tree.clock_at(e5), Some((180_000, 180_000)));
        assert_eq!(tree.clock_at(d5_variation), Some((180_000, 180_000)));

        // A node with its own recorded clock overrides inheritance.
        let mut clocked = MoveTree::with_clocks(Chess::default(), 180_000);
        let pos = clocked.position(0).clone();
        let m = "e2e4".parse::<UciMove>().unwrap().to_move(&pos).unwrap();
        let e4_clocked = clocked.play(0, m, Some((179_000, 180_000)));
        assert_eq!(clocked.clock_at(e4_clocked), Some((179_000, 180_000)));
    }

    #[test]
    fn pgn_round_trip_with_variation_and_clocks() {
        let pgn = "1. e4 {[%clk 0:02:59]} e5 {[%clk 0:02:58]} 2. Nf3 {[%clk 0:02:55]} \
                   (2. d4 {[%clk 0:02:50]} exd4 {[%clk 0:02:57]}) Nc6 {[%clk 0:02:53]}";
        let tree = MoveTree::from_pgn(pgn).expect("valid pgn");

        let segments = tree.flatten();
        assert_eq!(segments.len(), 3);
        let mainline_opening: Vec<&str> = segments[0].nodes.iter().map(|n| n.san.as_str()).collect();
        assert_eq!(mainline_opening, ["e4", "e5"]);
        let variation: Vec<&str> = segments[1].nodes.iter().map(|n| n.san.as_str()).collect();
        assert_eq!(variation, ["d4", "exd4"]);
        let resumed: Vec<&str> = segments[2].nodes.iter().map(|n| n.san.as_str()).collect();
        assert_eq!(resumed, ["Nf3", "Nc6"]);

        let e4_id = segments[0].nodes[0].id;
        assert_eq!(tree.clock_at(e4_id), Some((179_000, 0)));

        // Round trip through to_pgn/from_pgn.
        let re_pgn = tree.to_pgn();
        let round_tripped = MoveTree::from_pgn(&re_pgn).expect("re-parses own output");
        let rt_segments = round_tripped.flatten();
        assert_eq!(rt_segments.len(), segments.len());
        for (a, b) in rt_segments.iter().zip(segments.iter()) {
            assert_eq!(a.depth, b.depth);
            let a_sans: Vec<&str> = a.nodes.iter().map(|n| n.san.as_str()).collect();
            let b_sans: Vec<&str> = b.nodes.iter().map(|n| n.san.as_str()).collect();
            assert_eq!(a_sans, b_sans);
        }
    }

    #[test]
    fn from_pgn_rejects_unmatched_parens() {
        assert!(MoveTree::from_pgn("1. e4 (1. d4").is_err());
        assert!(MoveTree::from_pgn("1. e4) e5").is_err());
    }

    #[test]
    fn from_pgn_rejects_illegal_move() {
        assert!(MoveTree::from_pgn("1. e2e9").is_err());
    }

    #[test]
    fn from_uci_moves_builds_a_linear_tree_with_clocks() {
        let moves = vec!["e2e4".to_string(), "e7e5".to_string(), "g1f3".to_string()];
        let clocks = vec![
            Some((299_000, 300_000)),
            Some((299_000, 298_500)),
            None, // simulates an older row missing this move's clock
        ];
        let tree = MoveTree::from_uci_moves(&moves, &clocks, 300_000).unwrap();

        let segments = tree.flatten();
        assert_eq!(segments.len(), 1, "no variations expected");
        let sans: Vec<&str> = segments[0].nodes.iter().map(|n| n.san.as_str()).collect();
        assert_eq!(sans, ["e4", "e5", "Nf3"]);

        let e4_id = segments[0].nodes[0].id;
        let e5_id = segments[0].nodes[1].id;
        let nf3_id = segments[0].nodes[2].id;
        assert_eq!(tree.clock_at(e4_id), Some((299_000, 300_000)));
        assert_eq!(tree.clock_at(e5_id), Some((299_000, 298_500)));
        // Nf3 had no clock of its own — inherits from its parent (e5).
        assert_eq!(tree.clock_at(nf3_id), Some((299_000, 298_500)));

        // Root itself carries the seeded starting clock.
        assert_eq!(tree.clock_at(tree.root()), Some((300_000, 300_000)));
    }

    #[test]
    fn from_uci_moves_rejects_bad_move() {
        assert!(MoveTree::from_uci_moves(&["e2e9".to_string()], &[None], 300_000).is_err());
    }
}
