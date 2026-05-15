use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect},
};
use serde::Deserialize;
use tower_sessions::Session;

use crate::auth::{AuthBackend, AuthError, Credentials};

pub async fn github_login(
    State(backend): State<AuthBackend>,
    session: Session,
) -> impl IntoResponse {
    let (redirect_url, csrf_token) = backend
        .github_client
        .authorize_url(oauth2::CsrfToken::new_random)
        .add_scope(oauth2::Scope::new("user:email".to_string()))
        .url();

    session
        .insert("oauth_csrf_token", csrf_token.secret())
        .await
        .unwrap();

    Redirect::to(redirect_url.as_str())
}

#[derive(Deserialize)]
pub struct GitHubCallbackQuery {
    code: String,
    state: String,
}

pub async fn github_callback(
    Query(query): Query<GitHubCallbackQuery>,
    mut auth_session: axum_login::AuthSession<AuthBackend>,
    session: Session,
) -> impl IntoResponse {
    let csrf_token = match session.get::<String>("oauth_csrf_token").await.unwrap() {
        Some(token) => token,
        None => {
            tracing::error!("github callback: no csrf token in session");
            return Redirect::to("/login");
        }
    };

    if csrf_token != query.state {
        tracing::error!("github callback: csrf mismatch (session={csrf_token}, query={})", query.state);
        return Redirect::to("/login");
    }

    let credentials = Credentials::GitHubOAuth {
        code: query.code.clone(),
    };

    match auth_session.authenticate(credentials).await {
        Ok(Some(user)) => {
            auth_session.login(&user).await.unwrap();
            Redirect::to("/")
        }
        Ok(None) => {
            tracing::error!("github callback: authenticate returned None");
            Redirect::to("/login")
        }
        Err(e) => {
            tracing::error!("github callback: authenticate error: {e:?}");
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

pub async fn get_github_user(
    client: &reqwest::Client,
    jwt_token: &str,
) -> Result<GitHubUser, AuthError> {
    let user_response = client
        .get("https://api.github.com/user")
        .bearer_auth(&jwt_token)
        .send()
        .await?
        .error_for_status()?;

    let user: GitHubUser = user_response.json().await?;

    if user.email.is_none() {
        let emails_response = client
            .get("https://api.github.com/user/emails")
            .bearer_auth(&jwt_token)
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
