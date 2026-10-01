# chess-rs

A full-stack chess platform built with Rust, Leptos, and Axum.

## Structure

```
chess-rs/
├── crates/
│   ├── shared/     # Types shared between server and WASM client
│   └── web/        # Leptos full-stack app (SSR + WASM)
├── migrations/     # sqlx Postgres migrations
└── devenv.nix      # Dev environment
```

## Prerequisites

Install [nix](https://nixos.org/download/) and [devenv](https://devenv.sh/getting-started/):

```bash
nix-env -iA devenv -f https://github.com/NixOS/nixpkgs/tarball/nixpkgs-unstable
```

Optionally install [direnv](https://direnv.net/) to auto-activate the shell on `cd`:

```bash
echo 'eval "$(direnv hook zsh)"' >> ~/.zshrc  # or bash/fish equivalent
direnv allow
```

## Getting started

First, configure the environment. Every variable is read with `.expect(...)`
at startup, so a missing one panics before the server binds:

```bash
cp crates/web/.env.example crates/web/.env
```

Then fill in the four OAuth values — `.env.example` links to the pages that
issue them. The rest of the defaults work as-is against `devenv`'s services.

```bash
# enter dev shell (skip if using direnv)
devenv shell

# start Postgres and Redis
devenv up
```

In a separate terminal:

```bash
devenv shell

# run migrations
migrate

# start dev server with hot reload
dev
```

Open [http://localhost:3000](http://localhost:3000).

## Commands

| Command | Description |
|---|---|
| `dev` | Hot-reload dev server (`cargo leptos watch`) |
| `migrate` | Run pending database migrations |
| `check-all` | Typecheck both server and WASM targets |

## Tests

Postgres and Redis must be running (`devenv up`) — the `state` and `db` test
modules talk to both for real rather than mocking them. `#[sqlx::test]` gives
each DB test its own throwaway database.

```bash
cargo test -p shared                  # pure logic, no services needed
devenv shell -- cargo test -p web --features ssr
```

## Changing SQL

`sqlx` verifies every query against a live database at compile time, and
caches the results in `.sqlx/` so builds work without one (CI, Docker,
`SQLX_OFFLINE=true`). After adding or editing any `sqlx::query!` /
`query_as!` / `query_scalar!`, regenerate that cache and commit it, or the
next offline build fails:

```bash
devenv shell -- cargo sqlx prepare --workspace -- --features ssr --all-targets
```

`--all-targets` matters: without it, queries that only appear in `#[cfg(test)]`
code are left out of the cache.

## Production build

```bash
cargo leptos build --release
```

Outputs server binary to `target/release/web` and static assets to `target/site`.
