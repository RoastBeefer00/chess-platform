{ pkgs, ... }:

{
  packages = with pkgs; [
    flyctl
    sqlx-cli
    cargo-leptos
    flutter
    dart
    tailwindcss_4
    wasm-bindgen-cli
    binaryen
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
  };

  services.redis.enable = true;

  env = {
    DATABASE_URL = "postgresql://localhost:5433/chess_dev";
    REDIS_URL = "redis://localhost:6379";
    LEPTOS_OUTPUT_NAME = "web";
    LEPTOS_SITE_ROOT = "target/site";
    LEPTOS_SITE_ADDR = "127.0.0.1:3000";
    RUST_LOG = "web=debug,tower_http=info,tower_sessions=debug";
  };

  scripts = {
    dev.exec = "cargo leptos watch --release -P --project web --split";
    kill-dev.exec = "lsof -ti :3000 | xargs kill -9 2>/dev/null; echo 'done'";
    migrate.exec = "sqlx migrate run";
    flutter-dev.exec = "cd mobile && flutter run";
    check-all.exec = ''
      cargo check --workspace
      cargo check -p web --target wasm32-unknown-unknown --features hydrate
    '';
  };
}
