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

## Production build

```bash
cargo leptos build --release
```

Outputs server binary to `target/release/web` and static assets to `target/site`.
