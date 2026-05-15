#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() {
    use axum::{routing::get, Router};
    use axum_login::AuthManagerLayerBuilder;
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use sqlx::PgPool;
    use tower_http::trace::TraceLayer;
    use tower_sessions::{cookie::SameSite, SessionManagerLayer};
    use tower_sessions_sqlx_store::PostgresStore;
    use tracing::info;
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
    use web::app::*;
    use web::auth::{github_callback, github_login, google_callback, google_login};
    use web::state::AppState;

    dotenvy::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/.env")).ok();

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

    let pool = PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"))
        .await
        .unwrap();

    let session_store = PostgresStore::new(pool.clone());
    session_store.migrate().await.unwrap();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false) // set to true behind HTTPS in prod
        .with_same_site(SameSite::Lax); // required so the session cookie is sent on the OAuth callback redirect

    let app_state = AppState::new(leptos_options.clone(), pool).await;
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
                let games = app_state.games.clone();
                let backend = app_state.auth_backend.clone();
                move || {
                    provide_context(games.clone());
                    provide_context(backend.clone());
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
