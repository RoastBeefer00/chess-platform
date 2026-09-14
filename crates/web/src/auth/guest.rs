use axum::response::{IntoResponse, Redirect};
use axum_login::AuthSession;

use crate::auth::{AuthBackend, Credentials};

/// Unlike the OAuth providers, there's no external redirect round-trip to
/// protect (no CSRF token, no callback) — a single request creates a fresh
/// guest account and logs it in immediately.
#[tracing::instrument(skip_all)]
pub async fn guest_login(mut auth_session: AuthSession<AuthBackend>) -> impl IntoResponse {
    match auth_session.authenticate(Credentials::Guest).await {
        Ok(Some(user)) => match auth_session.login(&user).await {
            Ok(()) => Redirect::to("/"),
            Err(_) => {
                tracing::warn!("guest_login: session login failed");
                Redirect::to("/login")
            }
        },
        Ok(None) => {
            tracing::warn!("guest_login: authenticate returned None");
            Redirect::to("/login")
        }
        Err(e) => {
            tracing::warn!(error = ?e, "guest_login: authenticate error");
            Redirect::to("/login")
        }
    }
}
