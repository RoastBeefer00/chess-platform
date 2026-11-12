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

/// The one address from `/user/emails` we'll accept as an identity.
///
/// Both flags are required, and neither is redundant. `verified` alone isn't
/// enough — a GitHub account can hold several verified addresses, and picking
/// any of them makes the account's identity depend on list order. `primary`
/// alone isn't enough either: GitHub lets an unverified address be primary,
/// and an address nobody has proven they control is exactly what we must not
/// key an account on.
///
/// `None` means the account has no address meeting both conditions, which the
/// caller turns into `AuthError::NoVerifiedPrimaryEmail` rather than falling
/// back to a weaker pick.
pub fn pick_primary_verified(emails: Vec<GitHubEmail>) -> Option<String> {
    emails
        .into_iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email)
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
        match pick_primary_verified(emails) {
            Some(email) => Ok(GitHubUser {
                id: user.id,
                email: Some(email),
            }),
            None => Err(AuthError::NoVerifiedPrimaryEmail),
        }
    } else {
        Ok(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn email(addr: &str, primary: bool, verified: bool) -> GitHubEmail {
        GitHubEmail {
            email: addr.to_string(),
            primary,
            verified,
        }
    }

    #[test]
    fn picks_the_primary_verified_address() {
        let picked = pick_primary_verified(vec![
            email("alt@example.com", false, true),
            email("main@example.com", true, true),
        ]);
        assert_eq!(picked.as_deref(), Some("main@example.com"));
    }

    #[test]
    fn rejects_primary_but_unverified() {
        // GitHub permits this combination; keying an account on an address
        // nobody proved they control is the thing we're guarding against.
        let picked = pick_primary_verified(vec![email("main@example.com", true, false)]);
        assert_eq!(picked, None);
    }

    #[test]
    fn rejects_verified_but_not_primary() {
        // Several verified addresses is normal; without `primary` the pick
        // would depend on list order.
        let picked = pick_primary_verified(vec![
            email("one@example.com", false, true),
            email("two@example.com", false, true),
        ]);
        assert_eq!(picked, None);
    }

    #[test]
    fn empty_list_yields_none() {
        assert_eq!(pick_primary_verified(vec![]), None);
    }

    #[test]
    fn ignores_a_non_primary_match_before_the_real_one() {
        // Ordering must not matter: the primary+verified entry wins even
        // when noise sorts ahead of it.
        let picked = pick_primary_verified(vec![
            email("noise@example.com", false, false),
            email("alt@example.com", false, true),
            email("main@example.com", true, true),
            email("later@example.com", false, true),
        ]);
        assert_eq!(picked.as_deref(), Some("main@example.com"));
    }
}
