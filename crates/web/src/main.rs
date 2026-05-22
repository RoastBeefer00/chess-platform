#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::{routing::get, Router};
    use axum_login::AuthManagerLayerBuilder;
    use fred::prelude::*;
    use fred::types::config::{ConnectionConfig, PerformanceConfig};
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use tower_http::trace::TraceLayer;
    use tower_sessions::{cookie::SameSite, SessionManagerLayer};
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
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(env == "production") // set to true behind HTTPS in prod
        .with_same_site(SameSite::Lax); // required so the session cookie is sent on the OAuth callback redirect

    let app_state = AppState::new(leptos_options.clone(), pool, redis).await;
    let auth_layer =
        AuthManagerLayerBuilder::new(app_state.auth_backend.clone(), session_layer).build();

    let routes = generate_route_list(App);

    let app = Router::new()
        .route("/auth/github", get(github_login))
        .route("/auth/github/callback", get(github_callback))
        .route("/auth/google", get(google_login))
        .route("/auth/google/callback", get(google_callback))
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
        .layer(TraceLayer::new_for_http())
        .layer(auth_layer)
        .with_state(app_state);

    info!("listening on http://{}", &addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

#[cfg(not(feature = "ssr"))]
pub fn main() {
    // no client-side main function
    // unless we want this to work with e.g., Trunk for pure client-side testing
    // see lib.rs for hydration function instead
}
