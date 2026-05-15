use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect},
};
use tower_sessions::Session;

use crate::auth::{AuthBackend, Credentials};

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

    session
        .insert("oauth_csrf_token", csrf_token.secret())
        .await
        .unwrap();
    session.insert("oidc_nonce", nonce.secret()).await.unwrap();

    Redirect::to(redirect_url.as_str())
}

#[derive(serde::Deserialize)]
pub struct GoogleCallbackQuery {
    code: String,
    state: String,
}

pub async fn google_callback(
    Query(query): Query<GoogleCallbackQuery>,
    mut auth_session: axum_login::AuthSession<AuthBackend>,
    session: Session,
) -> impl IntoResponse {
    let csrf_token = match session.get::<String>("oauth_csrf_token").await.unwrap() {
        Some(token) => token,
        None => return Redirect::to("/login"),
    };

    if csrf_token != query.state {
        return Redirect::to("/login");
    }

    let nonce = match session.get::<String>("oidc_nonce").await.unwrap() {
        Some(n) => n,
        None => return Redirect::to("/login"),
    };

    let credentials = Credentials::GoogleOAuth {
        code: query.code.clone(),
        nonce,
    };

    match auth_session.authenticate(credentials).await {
        Ok(Some(user)) => {
            auth_session.login(&user).await.unwrap();
            Redirect::to("/")
        }
        Ok(None) | Err(_) => Redirect::to("/login"),
    }
}
