{ pkgs, ... }:

{
  packages = with pkgs; [
    flyctl
    sqlx-cli
    cargo-leptos
    flutter
    dart
    tailwindcss_4
    # Pinned to match `crates/web/Cargo.toml`'s exact `wasm-bindgen = "=0.2.117"`
    # — the CLI's bindgen schema version must match the Rust crate's exactly
    # or `cargo leptos build`/`watch` fails outright. Plain `wasm-bindgen-cli`
    # tracks whatever nixpkgs currently has (drifted to 0.2.121 after
    # `devenv update`); bumping the Rust-side pin instead cascades through
    # js-sys/web-sys/wasm-bindgen-futures and whatever else transitively
    # pins them, which is a much bigger, riskier change than pinning the CLI.
    wasm-bindgen-cli_0_2_117
    binaryen
    libiconv
  ];

  languages.rust = {
    enable = true;
    channel = "stable";
    targets = [ "wasm32-unknown-unknown" ];
  };

  services.postgres = {
    enable = true;
    initialDatabases = [ { name = "chess_dev"; } ];
    listen_addresses = "127.0.0.1";
    # Pinned rather than left to devenv's own default: that default has
    # drifted at least once already (silently, between 5432 and 5433 across
    # `devenv update`s — visible in old vs. new `.devenv/shell-*.sh`'s
    # `PGPORT`), and unlike redis's port (see below), devenv *does*
    # regenerate postgresql.conf's `port` line from this value on every
    # `devenv up`, even against an existing data directory — "skipping
    # initialization" only skips `initdb`, not conf regeneration. So this
    # single value, matched by `DATABASE_URL` below, is now the one source
    # of truth for both.
    port = 5432;
  };

  services.redis = {
    enable = true;
    # `port` alone doesn't work: devenv's redis module never wires
    # `services.redis.port` into the generated redis.conf, which always
    # bakes in a literal `port 6380` regardless of this setting (confirmed
    # directly, both before and after a `devenv update` bumping devenv's
    # own locked module version — `port = 6379;` alone still starts redis
    # on 6380 every time, freshly, no stale state involved). `extraConfig`
    # appends after that broken line, and redis.conf takes the *last*
    # occurrence of a directive — so this line is what actually wins.
    # Verified via `redis-cli -p 6379 ping` → PONG.
    port = 6379;
    extraConfig = "port 6379";
  };

  env = {
    # Port must match `services.postgres.port` above exactly — devenv
    # regenerates `.devenv/state/postgres/postgresql.conf`'s `port` line
    # from that value on every `devenv up` (confirmed directly: manually
    # editing the conf file gets silently overwritten back on the next
    # `up`), so that Nix value, not this literal, is the actual source of
    # truth. If connections ever start timing out again, check
    # `services.postgres.port` was actually changed and this was updated to
    # match, rather than editing the persisted conf — that edit won't stick.
    #
    # The `roastbeefer@` user is required, not optional: sqlx-cli 0.9.0
    # (picked up by the same `devenv update` as the redis/wasm-bindgen
    # fixes above) defaults an unqualified DATABASE_URL to a role literally
    # named "anonymous" instead of falling back to the OS user the way
    # `psql`/libpq do — `sqlx migrate run` fails outright
    # (`role "anonymous" does not exist`) without an explicit username here.
    DATABASE_URL = "postgresql://roastbeefer@localhost:5432/chess_dev";
    REDIS_URL = "redis://localhost:6379";
    LEPTOS_OUTPUT_NAME = "web";
    LEPTOS_SITE_ROOT = "target/site";
    LEPTOS_SITE_ADDR = "127.0.0.1:3000";
    RUST_LOG = "web=debug,tower_http=info,tower_sessions=debug";
  };

  scripts = {
    dev.exec = "cargo leptos watch --project web --split";
    kill-dev.exec = "lsof -ti :3000 | xargs kill -9 2>/dev/null; echo 'done'";
    migrate.exec = "sqlx migrate run";
    flutter-dev.exec = "cd mobile && flutter run";
    check-all.exec = ''
      cargo check --workspace
      cargo check -p web --target wasm32-unknown-unknown --features hydrate
    '';
    clippy.exec = ''
      cargo clippy --workspace --all-targets --features "web/ssr" -- -D warnings
      cargo clippy -p web --target wasm32-unknown-unknown --features hydrate -- -D warnings
    '';
  };
}
