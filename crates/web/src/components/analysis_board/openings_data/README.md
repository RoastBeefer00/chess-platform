# Opening database

`a.tsv`–`e.tsv` (one file per ECO volume, ~3,800 named positions total) are
[lichess-org/chess-openings](https://github.com/lichess-org/chess-openings),
the same dataset lichess.org itself uses for opening names in analysis.

**License: CC0 1.0 Universal (public domain dedication)** — no attribution
required. Obtained via the vendored copy bundled in the
[`esca`](https://crates.io/crates/esca) crate's `openings` feature (MIT-licensed
code, this data unaffected) rather than fetched directly, since this
environment has no general network access — `esca` itself credits the
original source in its README's acknowledgments.

Parsed once, lazily, by `../openings.rs` at first lookup — no build step,
no external crate dependency at runtime.
