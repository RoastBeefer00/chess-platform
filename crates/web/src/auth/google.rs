use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect},
};
use tower_sessions::Session;

use crate::auth::{AuthBackend, Credentials};

#[tracing::instrument(skip_all)]
pub async fn google_login(
    State(backend): State<AuthBackend>,
    session: Session,
) -> impl IntoResponse {
    let (redirect_url, csrf_token, nonce) = backend
        .google_client
        .authorize_url(
            openidconnect::AuthenticationFlow::<openidconnect::core::CoreResponseType>::AuthorizationCode,
            openidconnect::CsrfToken::new_random,
            openidconnect::Nonce::new_random,
        )
        .add_scope(openidconnect::Scope::new("openid".to_string()))
        .add_scope(openidconnect::Scope::new("email".to_string()))
        .add_scope(openidconnect::Scope::new("profile".to_string()))
        .url();

    if session
        .insert("oauth_csrf_token", csrf_token.secret())
        .await
        .is_err()
        || session
            .insert("oidc_nonce", nonce.secret())
            .await
            .is_err()
    {
        tracing::warn!("google_login: failed to persist CSRF/nonce");
        return Redirect::to("/login");
    }

    Redirect::to(redirect_url.as_str())
}

#[derive(serde::Deserialize)]
pub struct GoogleCallbackQuery {
    code: String,
    state: String,
}

#[tracing::instrument(skip_all)]
pub async fn google_callback(
    Query(query): Query<GoogleCallbackQuery>,
    mut auth_session: axum_login::AuthSession<AuthBackend>,
    session: Session,
) -> impl IntoResponse {
    let csrf_token = match session.get::<String>("oauth_csrf_token").await {
        Ok(Some(token)) => token,
        Ok(None) => {
            tracing::warn!("google_callback: no csrf token in session");
            return Redirect::to("/login");
        }
        Err(_) => {
            tracing::warn!("google_callback: session store error reading csrf token");
            return Redirect::to("/login");
        }
    };

    if csrf_token != query.state {
        tracing::warn!("google_callback: csrf mismatch");
        return Redirect::to("/login");
    }

    let nonce = match session.get::<String>("oidc_nonce").await {
        Ok(Some(n)) => n,
        Ok(None) => {
            tracing::warn!("google_callback: no nonce in session");
            return Redirect::to("/login");
        }
        Err(_) => {
            tracing::warn!("google_callback: session store error reading nonce");
            return Redirect::to("/login");
        }
    };

    let credentials = Credentials::GoogleOAuth {
        code: query.code.clone(),
        nonce,
    };

    match auth_session.authenticate(credentials).await {
        Ok(Some(user)) => match auth_session.login(&user).await {
            Ok(()) => Redirect::to("/"),
            Err(_) => {
                tracing::warn!("google_callback: session login failed");
                Redirect::to("/login")
            }
        },
        Ok(None) => {
            tracing::warn!("google_callback: authenticate returned None");
            Redirect::to("/login")
        }
        Err(e) => {
            tracing::warn!(error = ?e, "google_callback: authenticate error");
            Redirect::to("/login")
        }
    }
}
