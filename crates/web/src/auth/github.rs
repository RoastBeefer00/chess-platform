use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect},
};
use serde::Deserialize;
use tower_sessions::Session;

use crate::auth::{AuthBackend, AuthError, Credentials};

#[tracing::instrument(skip_all)]
pub async fn github_login(
    State(backend): State<AuthBackend>,
    session: Session,
) -> impl IntoResponse {
    let (redirect_url, csrf_token) = backend
        .github_client
        .authorize_url(oauth2::CsrfToken::new_random)
        .add_scope(oauth2::Scope::new("user:email".to_string()))
        .url();

    if session
        .insert("oauth_csrf_token", csrf_token.secret())
        .await
        .is_err()
    {
        tracing::warn!("github_login: failed to persist CSRF token");
        return Redirect::to("/login");
    }

    Redirect::to(redirect_url.as_str())
}

#[derive(Deserialize)]
pub struct GitHubCallbackQuery {
    code: String,
    state: String,
}

#[tracing::instrument(skip_all)]
pub async fn github_callback(
    Query(query): Query<GitHubCallbackQuery>,
    mut auth_session: axum_login::AuthSession<AuthBackend>,
    session: Session,
) -> impl IntoResponse {
    let csrf_token = match session.get::<String>("oauth_csrf_token").await {
        Ok(Some(token)) => token,
        Ok(None) => {
            tracing::warn!("github_callback: no csrf token in session");
            return Redirect::to("/login");
        }
        Err(_) => {
            tracing::warn!("github_callback: session store error reading csrf token");
            return Redirect::to("/login");
        }
    };

    if csrf_token != query.state {
        // Intentionally omit both token values from the log.
        tracing::warn!("github_callback: csrf mismatch");
        return Redirect::to("/login");
    }

    let credentials = Credentials::GitHubOAuth {
        code: query.code.clone(),
    };

    match auth_session.authenticate(credentials).await {
        Ok(Some(user)) => match auth_session.login(&user).await {
            Ok(()) => Redirect::to("/"),
            Err(_) => {
                tracing::warn!("github_callback: session login failed");
                Redirect::to("/login")
            }
        },
        Ok(None) => {
            tracing::warn!("github_callback: authenticate returned None");
            Redirect::to("/login")
        }
        Err(e) => {
            tracing::warn!(error = ?e, "github_callback: authenticate error");
            Redirect::to("/login")
        }
    }
}

#[derive(Deserialize)]
pub struct GitHubUser {
    pub id: i64,
    pub email: Option<String>,
}

#[derive(Deserialize)]
pub struct GitHubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

#[tracing::instrument(skip_all)]
pub async fn get_github_user(
    client: &reqwest::Client,
    jwt_token: &str,
) -> Result<GitHubUser, AuthError> {
    let user_response = client
        .get("https://api.github.com/user")
        .bearer_auth(jwt_token)
        .send()
        .await?
        .error_for_status()?;

    let user: GitHubUser = user_response.json().await?;

    if user.email.is_none() {
        let emails_response = client
            .get("https://api.github.com/user/emails")
            .bearer_auth(jwt_token)
            .send()
            .await?
            .error_for_status()?;

        let emails: Vec<GitHubEmail> = emails_response.json().await?;
        if let Some(primary_email) = emails.into_iter().find(|e| e.primary && e.verified) {
            Ok(GitHubUser {
                id: user.id,
                email: Some(primary_email.email),
            })
        } else {
            Err(AuthError::NoVerifiedPrimaryEmail)
        }
    } else {
        Ok(user)
    }
}
