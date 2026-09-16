#![recursion_limit = "512"]

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::extract::Request;
    use axum::http::{header, HeaderValue, StatusCode};
    use axum::middleware::{from_fn, Next};
    use axum::response::Response;
    use axum::{routing::get, Router};
    use axum_login::AuthManagerLayerBuilder;
    use fred::clients::SubscriberClient;
    use fred::prelude::*;
    use fred::types::config::{ConnectionConfig, PerformanceConfig};
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use sqlx::postgres::PgPoolOptions;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;
    use time::Duration as TimeDuration;
    use tower_governor::{
        governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
    };
    use tower_http::limit::RequestBodyLimitLayer;
    use tower_http::set_header::SetResponseHeaderLayer;
    use tower_http::trace::TraceLayer;
    use tower_sessions::{cookie::SameSite, Expiry, SessionManagerLayer};
    use tower_sessions_redis_store::RedisStore;

    use tracing::info;
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
    use web::app::*;
    use web::auth::{github_callback, github_login, google_callback, google_login, guest_login};
    use web::state::AppState;

    dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            "web=debug,tower_http=info"
                .parse()
                .expect("hardcoded EnvFilter must parse")
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let conf = get_configuration(None).expect("failed to load Leptos configuration");
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .min_connections(1)
        .max_lifetime(Some(Duration::from_secs(15 * 60)))
        .idle_timeout(Some(Duration::from_secs(5 * 60)))
        .acquire_timeout(Duration::from_secs(10))
        .test_before_acquire(true)
        .connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"))
        .await
        .expect("failed to connect to Postgres");

    let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis_config = Config::from_url(&redis_url).expect("invalid REDIS_URL");
    let redis_conn_cfg = ConnectionConfig {
        connection_timeout: Duration::from_secs(10),
        internal_command_timeout: Duration::from_secs(10),
        ..Default::default()
    };
    let redis_policy = ReconnectPolicy::new_exponential(0, 100, 30_000, 2);
    let redis = Pool::new(
        redis_config.clone(),
        Some(PerformanceConfig::default()),
        Some(redis_conn_cfg.clone()),
        Some(redis_policy.clone()),
        6,
    )
    .expect("failed to build Redis pool");
    redis.connect();
    redis
        .wait_for_connect()
        .await
        .expect("Redis pool failed initial connect");
    // Dedicated subscriber connection for cross-instance friend/matchmaking
    // pub/sub delivery (see `RedisClient::new`) — a pool round-robins
    // regular commands across connections, which is wrong for a long-lived
    // SUBSCRIBE; this needs its own connection that stays subscribed.
    let redis_subscriber = SubscriberClient::new(
        redis_config,
        Some(PerformanceConfig::default()),
        Some(redis_conn_cfg),
        Some(redis_policy),
    );
    let session_store = RedisStore::new(redis.clone());
    let env = std::env::var("ENV").expect("ENV must be set to 'development' or 'production'");
    let is_prod = env == "production";
    let session_layer = SessionManagerLayer::new(session_store)
        .with_name("chess_session")
        .with_http_only(true)
        .with_secure(is_prod) // set to true behind HTTPS in prod
        .with_same_site(SameSite::Lax) // required so the session cookie is sent on the OAuth callback redirect
        .with_expiry(Expiry::OnInactivity(TimeDuration::days(14)));

    let app_state = AppState::new(leptos_options.clone(), pool, redis, redis_subscriber).await;

    // Heartbeat-aware reaper (see `AppState::reconcile_stale_active_games`) —
    // run once before accepting traffic, so stale rows never resurface as
    // false "you have an active game" banners after this instance's own
    // restart, then periodically, since a *peer* instance can go quiet at
    // any time, not just when this one happens to be booting.
    if let Err(err) = app_state.reconcile_stale_active_games().await {
        tracing::error!(?err, "failed to reconcile stale active games at startup");
    }
    {
        const RECONCILE_INTERVAL: Duration = Duration::from_secs(60);
        let app_state = app_state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(RECONCILE_INTERVAL);
            tick.tick().await; // skip the immediate first tick — just ran above
            loop {
                tick.tick().await;
                if let Err(err) = app_state.reconcile_stale_active_games().await {
                    tracing::error!(?err, "failed to reconcile stale active games");
                }
            }
        });
    }

    // Refresh Google's OIDC metadata (JWKS signing keys) every 6 hours so that
    // key rotation doesn't silently break logins until the next restart.
    // Startup already ran discovery, so skip the first tick.
    {
        let refresh_backend = app_state.auth_backend.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(6 * 60 * 60));
            tick.tick().await; // skip the immediate first tick
            loop {
                tick.tick().await;
                refresh_backend.refresh_google_metadata().await;
            }
        });
    }

    let auth_layer =
        AuthManagerLayerBuilder::new(app_state.auth_backend.clone(), session_layer).build();

    let routes = generate_route_list(App);

    // Rate limiting is scoped to dynamic paths only:
    //   /auth/*  — strict (OAuth init + callback), applied via nested Router below
    //   /api/*   — moderate (server fn POSTs), applied via path-filter middleware below
    // Static assets and page renders are unlimited. SmartIpKeyExtractor trusts
    // X-Forwarded-For when present and falls back to ConnectInfo peer address.
    // If we deploy directly without a trusted proxy, swap to PeerIpKeyExtractor
    // — otherwise a client can spoof the header to bypass limits.
    let oauth_governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(6) // ~10/min sustained with a small burst
            .burst_size(5)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("invalid OAuth governor config"),
    );
    let api_governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(2) // 120/min sustained
            .burst_size(30)
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("invalid API governor config"),
    );

    // Background task to free per-IP state for IPs we haven't seen in a while.
    let cleanup_governors = [oauth_governor.clone(), api_governor.clone()];
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            for g in cleanup_governors.iter() {
                g.limiter().retain_recent();
            }
        }
    });

    // `cargo-leptos` doesn't content-hash the WASM/JS bundle filenames
    // (output-name = "web" — always `web.wasm`, `chunk_N.wasm`, etc.), so a
    // rebuild reuses the exact same URL for different content. Without an
    // explicit header, browsers apply heuristic caching against
    // `Last-Modified` and can keep serving a stale (pre-rebuild) bundle
    // after a normal reload. `no-cache` still lets the browser cache the
    // response but forces revalidation (conditional GET) on every request,
    // so a rebuild is always picked up on the next load — in both dev and
    // prod, since neither hashes these filenames today.
    let no_cache_pkg = from_fn(|req: Request, next: Next| async move {
        let is_pkg = req.uri().path().starts_with("/pkg/");
        let mut res = next.run(req).await;
        if is_pkg {
            res.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        }
        res
    });

    // Opposite policy for the vendored Stockfish engine under `/engine/` —
    // unlike the app's own WASM, these files only change when someone
    // deliberately swaps in a new build (see public/engine/README.md), so
    // they're safe to cache hard rather than revalidate on every load. This
    // matters more here: the engine WASM is ~7MB, multiple times the size
    // of everything else on the page combined.
    let long_cache_engine = from_fn(|req: Request, next: Next| async move {
        let is_engine = req.uri().path().starts_with("/engine/");
        let mut res = next.run(req).await;
        if is_engine {
            res.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=604800, immutable"),
            );
        }
        res
    });

    // Middleware: rate-limit only paths under `/api/*`. Uses the same
    // SmartIpKeyExtractor logic so behavior is consistent with the OAuth
    // GovernorLayer. Static assets and page renders pass through untouched.
    let api_key_extractor = SmartIpKeyExtractor;
    let api_limiter = api_governor.limiter().clone();
    let api_rate_limit = from_fn(move |req: Request, next: Next| {
        let limiter = api_limiter.clone();
        let extractor = api_key_extractor;
        async move {
            if req.uri().path().starts_with("/api/") {
                use tower_governor::key_extractor::KeyExtractor;
                match extractor.extract(&req) {
                    Ok(key) => {
                        if limiter.check_key(&key).is_err() {
                            return Response::builder()
                                .status(StatusCode::TOO_MANY_REQUESTS)
                                .body(axum::body::Body::from("rate limited"))
                                .unwrap();
                        }
                    }
                    Err(_) => {
                        // No key (no IP at all) — let it through; we don't
                        // want to block legitimate traffic on extractor edge cases.
                    }
                }
            }
            next.run(req).await
        }
    });

    // Middleware: route a game-websocket connect request to whichever
    // instance actually owns that game's in-memory `GameRoom`, once more
    // than one instance can be running (see the N-instance statelessness
    // plan). `ws_session.rs`'s client appends `game_id` to the connect
    // URL specifically so this can run BEFORE the WebSocket upgrade
    // completes — `fly-replay` only works pre-upgrade; per Fly's own docs,
    // "an application returning fly-replay headers should not negotiate a
    // web socket upgrade itself." A miss (no `game_id`, malformed, no
    // ownership record, or the record names this instance) just falls
    // through to the normal handler — a truly-unknown/expired game still
    // gets today's "game not found" from inside it.
    let game_route_redis = app_state.redis_client.clone();
    let this_instance = std::env::var("FLY_MACHINE_ID").unwrap_or_else(|_| "local".to_string());
    let game_route = from_fn(move |req: Request, next: Next| {
        let redis = game_route_redis.clone();
        let this_instance = this_instance.clone();
        async move {
            if req.uri().path().starts_with("/api/game_websocket") {
                let game_id = req
                    .uri()
                    .query()
                    .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("game_id=")))
                    .and_then(|v| uuid::Uuid::parse_str(v).ok());
                if let Some(game_id) = game_id {
                    if let Some(owner) = redis.active_game_owner(game_id).await {
                        if owner != this_instance {
                            return Response::builder()
                                .status(StatusCode::OK)
                                .header("fly-replay", format!("instance={owner}"))
                                .body(axum::body::Body::empty())
                                .unwrap();
                        }
                    }
                }
            }
            next.run(req).await
        }
    });

    let oauth_routes = Router::new()
        .route("/auth/github", get(github_login))
        .route("/auth/github/callback", get(github_callback))
        .route("/auth/google", get(google_login))
        .route("/auth/google/callback", get(google_callback))
        // Guest creation is a real INSERT with no external provider
        // round-trip to naturally bottleneck it (unlike OAuth), so it needs
        // this rate limit at least as much as the OAuth routes do — more,
        // arguably, since nothing else stands between a script and a flood
        // of guest rows.
        .route("/auth/guest", get(guest_login))
        .layer(GovernorLayer::new(oauth_governor));

    // CSP allowances:
    //   wasm-unsafe-eval — Leptos hydrate WASM bundle
    //   'unsafe-eval' — wasm-bindgen's generated JS glue uses `new Function(...)`
    //     in spots; without this, dynamic view rendering (e.g. modal mounts)
    //     throws an EvalError at runtime in some Leptos code paths.
    //   script-src 'unsafe-inline' — Leptos emits inline hydration <script> tags;
    //     XSS protection then relies on Leptos's auto-escaping (default for view!).
    //     Avoid rendering user-controlled strings as raw HTML.
    //   style-src 'unsafe-inline' — Tailwind injects inline styles
    //   img-src https: data: — OAuth provider avatars + inline SVGs
    //   connect-src ws: wss: — game/matchmaking WebSocket
    //   frame-ancestors 'none' — clickjacking protection (doubles X-Frame-Options)
    let csp = "default-src 'self'; \
               script-src 'self' 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'; \
               style-src 'self' 'unsafe-inline'; \
               img-src 'self' data: https:; \
               connect-src 'self' ws: wss:; \
               font-src 'self' data:; \
               frame-ancestors 'none'; \
               base-uri 'self'; \
               form-action 'self'";

    let mut app = Router::new()
        .merge(oauth_routes)
        .leptos_routes_with_context(
            &app_state,
            routes,
            {
                let state = app_state.clone();
                move || {
                    provide_context(state.clone());
                }
            },
            {
                let leptos_options = leptos_options.clone();
                move || shell(leptos_options.clone())
            },
        )
        .fallback(leptos_axum::file_and_error_handler::<AppState, _>(shell))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(csp),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        // 1 MiB request body cap. Server functions and Leptos page renders fit
        // well within this; the WebSocket upgrade is a separate path. If we
        // later need a per-WS frame limit, switch to a hand-rolled axum WS
        // route with `max_message_size`.
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .layer(no_cache_pkg)
        .layer(long_cache_engine)
        .layer(api_rate_limit)
        .layer(game_route)
        .layer(TraceLayer::new_for_http())
        .layer(auth_layer);

    if is_prod {
        app = app.layer(SetResponseHeaderLayer::if_not_present(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ));
    }

    let shutdown_app_state = app_state.clone();
    let app = app.with_state(app_state);

    info!("listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind site_addr");

    // Bounded pause before actually exiting on SIGINT/SIGTERM (Fly sends one
    // of these on every stop — autostop-on-idle, deploys, host migrations —
    // see fly.toml's kill_timeout, raised specifically to give this room).
    // Without ANY signal handler at all (the previous state of this file),
    // the OS default disposition terminates the process immediately, with
    // zero warning to whatever's currently in flight — including a
    // just-finished game's finalize write (or a flag-fall's, or an ordinary
    // per-move persist), fire-and-forget or not. This is what turned a game
    // that had just been decided by timeout into a silently-lost result,
    // later misread by the reconciliation reaper as a genuinely abandoned
    // game. This grace period doesn't make that race impossible — a kill at
    // the exact wrong instant can still land — but it turns a window of
    // "however long it takes the OS to schedule the kill, often near-zero"
    // into one that reliably covers a single fast Postgres round trip.
    const SHUTDOWN_GRACE_PERIOD: Duration = Duration::from_secs(10);
    async fn shutdown_signal() {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = sigint.recv() => tracing::warn!("received SIGINT"),
            _ = sigterm.recv() => tracing::warn!("received SIGTERM"),
        }
    }

    tokio::select! {
        result = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        ) => {
            result.expect("axum::serve exited with error");
        }
        _ = shutdown_signal() => {
            tracing::warn!(
                grace_period_secs = SHUTDOWN_GRACE_PERIOD.as_secs(),
                "shutdown signal received — pausing before exit so in-flight \
                 writes get a real chance to land instead of being cut off \
                 with no warning at all",
            );
            tokio::time::sleep(SHUTDOWN_GRACE_PERIOD).await;
            tracing::info!("shutdown grace period elapsed, exiting");

            // Release this instance's ownership of every game it still
            // holds. Without this, a successor instance has to wait out
            // the up-to-30s `active_games:{id}` TTL before adoption
            // (`AppState::adopt_game`) even notices the game is ownerless —
            // during a routine deploy, where the old process is exiting
            // cleanly, there's no reason to wait: releasing here turns the
            // handover into a sub-second gap instead.
            let games = shutdown_app_state.games.lock().await;
            for (game_id, room) in games.iter() {
                if matches!(room.lock().await.status, shared::GameStatus::Ongoing) {
                    shutdown_app_state.redis_client.active_game_remove(*game_id).await;
                }
            }
            drop(games);
        }
    }
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
}
