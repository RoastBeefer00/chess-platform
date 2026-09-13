Vendored from [nmrugg/stockfish.js](https://github.com/nmrugg/stockfish.js) v18.0.8
(npm package `stockfish`, `bin/stockfish-18-lite-single.{js,wasm}`) — the "lite
single-threaded" build: NNUE net embedded, no `SharedArrayBuffer` /
Cross-Origin-Opener-Policy / Cross-Origin-Embedder-Policy headers required, runs
in a plain Web Worker. See `crates/web/src/components/analysis_board/engine.rs`
for the Rust side.

License: GPLv3, see `LICENSE.txt` (vendored from the same package's `Copying.txt`).

To update: download `stockfish-N-lite-single.js` + `.wasm` from a newer
[release](https://github.com/nmrugg/stockfish.js/releases) or the `stockfish`
npm package's `bin/`, drop them in here (same two-file naming), and update the
path referenced in `engine.rs`.
