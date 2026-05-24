#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::extract::Request;
    use axum::http::{header, HeaderValue, StatusCode};
    use axum::middleware::{from_fn, Next};
    use axum::response::Response;
    use axum::{routing::get, Router};
    use axum_login::AuthManagerLayerBuilder;
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
    use web::auth::{github_callback, github_login, google_callback, google_login};
    use web::state::AppState;

    dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "web=debug,tower_http=info".parse().unwrap()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let conf = get_configuration(None).unwrap();
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
        .unwrap();

    let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis_config = Config::from_url(&redis_url).unwrap();
    let redis_conn_cfg = ConnectionConfig {
        connection_timeout: Duration::from_secs(10),
        internal_command_timeout: Duration::from_secs(10),
        ..Default::default()
    };
    let redis_policy = ReconnectPolicy::new_exponential(0, 100, 30_000, 2);
    let redis = Pool::new(
        redis_config,
        Some(PerformanceConfig::default()),
        Some(redis_conn_cfg),
        Some(redis_policy),
        6,
    )
    .unwrap();
    redis.connect();
    redis.wait_for_connect().await.unwrap();
    let session_store = RedisStore::new(redis.clone());
    let env = std::env::var("ENV").expect("ENV must be set to 'development' or 'production'");
    let is_prod = env == "production";
    let session_layer = SessionManagerLayer::new(session_store)
        .with_name("chess_session")
        .with_http_only(true)
        .with_secure(is_prod) // set to true behind HTTPS in prod
        .with_same_site(SameSite::Lax) // required so the session cookie is sent on the OAuth callback redirect
        .with_expiry(Expiry::OnInactivity(TimeDuration::days(14)));

    let app_state = AppState::new(leptos_options.clone(), pool, redis).await;
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

    let oauth_routes = Router::new()
        .route("/auth/github", get(github_login))
        .route("/auth/github/callback", get(github_callback))
        .route("/auth/google", get(google_login))
        .route("/auth/google/callback", get(google_callback))
        .layer(GovernorLayer::new(oauth_governor));

    // CSP allowances:
    //   wasm-unsafe-eval — Leptos hydrate WASM bundle
    //   script-src 'unsafe-inline' — Leptos emits inline hydration <script> tags;
    //     XSS protection then relies on Leptos's auto-escaping (default for view!).
    //     Avoid rendering user-controlled strings as raw HTML.
    //   style-src 'unsafe-inline' — Tailwind injects inline styles
    //   img-src https: data: — OAuth provider avatars + inline SVGs
    //   connect-src ws: wss: — game/matchmaking WebSocket
    //   frame-ancestors 'none' — clickjacking protection (doubles X-Frame-Options)
    let csp = "default-src 'self'; \
               script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; \
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
        .layer(api_rate_limit)
        .layer(TraceLayer::new_for_http())
        .layer(auth_layer);

    if is_prod {
        app = app.layer(SetResponseHeaderLayer::if_not_present(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ));
    }

    let app = app.with_state(app_state);

    info!("listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
}
