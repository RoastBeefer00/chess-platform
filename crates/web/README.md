# web

Leptos 0.8 full-stack app. Compiles to two targets:

| Target | Feature flag | Output |
|---|---|---|
| Axum server | `--features ssr` | SSR + WebSocket handler binary |
| WASM bundle | `--features hydrate` | Client-side hydration bundle |

`cargo-leptos` orchestrates both targets via `[package.metadata.leptos]` in `Cargo.toml`.

## Structure

```
src/
├── app.rs          # Root component, router, shell
├── main.rs         # Server entry point (ssr)
├── lib.rs          # WASM entry point (hydrate)
├── components/     # Shared UI components (Nav, etc.)
└── pages/          # Route-level page components
```

## Dev

From the workspace root:

```bash
dev        # cargo leptos watch --project web
check-all  # typecheck both targets
```

## Feature gates

SSR-only code (DB queries, WebSocket handlers) must be gated:

```rust
#[cfg(feature = "ssr")]
async fn server_only_fn() { ... }
```

WASM target is `wasm32-unknown-unknown` — anything in the `hydrate` path must be WASM-compatible.
