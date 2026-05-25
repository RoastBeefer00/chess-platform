use axum_login::{AuthUser, AuthnBackend, UserId};
use oauth2::{
    basic::{
        BasicClient, BasicErrorResponseType, BasicRevocationErrorResponse,
        BasicTokenIntrospectionResponse, BasicTokenResponse,
    },
    Client, EndpointNotSet, EndpointSet, StandardErrorResponse, StandardRevocableToken,
    TokenResponse,
};
use openidconnect::{
    core::{CoreClient, CoreProviderMetadata},
    ClientId, ClientSecret, EndpointMaybeSet, EndpointSet as OidcEndpointSet, IssuerUrl,
    RedirectUrl,
};

type GoogleClient = CoreClient<
    OidcEndpointSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;
use serde::{Deserialize, Serialize};
use sqlx::{query_as, PgPool};
use uuid::Uuid;

use crate::auth::{get_github_user, AuthError};

type OAuthClient = Client<
    StandardErrorResponse<BasicErrorResponseType>,
    BasicTokenResponse,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointSet,
>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub country: Option<String>,
    pub created_at: time::OffsetDateTime,
}

impl AuthUser for User {
    type Id = Uuid;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn session_auth_hash(&self) -> &[u8] {
        // OAuth-only auth: no password to invalidate sessions against.
        // Use the user's stable UUID bytes — sessions are invalidated by
        // explicit logout, not by hash rotation.
        self.id.as_bytes()
    }
}

pub enum Credentials {
    GitHubOAuth { code: String },
    GoogleOAuth { code: String, nonce: String },
}

#[derive(Clone, Debug)]
pub struct AuthBackend {
    pool: PgPool,
    http_client: reqwest::Client,
    pub github_client: OAuthClient,
    pub google_client: GoogleClient,
}

impl AuthBackend {
    #[tracing::instrument(skip_all)]
    pub async fn new(pool: PgPool, http_client: reqwest::Client) -> Self {
        let github_client_id =
            std::env::var("GITHUB_CLIENT_ID").expect("GITHUB_CLIENT_ID must be set");
        let github_client_secret =
            std::env::var("GITHUB_CLIENT_SECRET").expect("GITHUB_CLIENT_SECRET must be set");
        let github_redirect_uri =
            std::env::var("GITHUB_REDIRECT_URI").expect("GITHUB_REDIRECT_URI must be set");
        let github_client = BasicClient::new(oauth2::ClientId::new(github_client_id))
            .set_client_secret(oauth2::ClientSecret::new(github_client_secret))
            .set_redirect_uri(
                oauth2::RedirectUrl::new(github_redirect_uri).expect("Invalid redirect URI"),
            )
            .set_auth_uri(
                oauth2::AuthUrl::new("https://github.com/login/oauth/authorize".to_string())
                    .expect("Invalid authorization endpoint URL"),
            )
            .set_token_uri(
                oauth2::TokenUrl::new("https://github.com/login/oauth/access_token".to_string())
                    .expect("Invalid token endpoint URL"),
            );

        let google_client_id =
            std::env::var("GOOGLE_CLIENT_ID").expect("GOOGLE_CLIENT_ID must be set");
        let google_client_secret =
            std::env::var("GOOGLE_CLIENT_SECRET").expect("GOOGLE_CLIENT_SECRET must be set");
        let google_redirect_uri =
            std::env::var("GOOGLE_REDIRECT_URI").expect("GOOGLE_REDIRECT_URI must be set");
        let provider_metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new("https://accounts.google.com".to_string()).expect("Invalid issuer URL"),
            &http_client,
        )
        .await
        .expect("Failed to discover Google OIDC metadata");
        let google_client = CoreClient::from_provider_metadata(
            provider_metadata,
            ClientId::new(google_client_id),
            Some(ClientSecret::new(google_client_secret)),
        )
        .set_redirect_uri(RedirectUrl::new(google_redirect_uri).expect("Invalid redirect URI"));

        AuthBackend {
            pool,
            http_client,
            github_client,
            google_client,
        }
    }
}

impl AuthnBackend for AuthBackend {
    type User = User;
    type Credentials = Credentials;
    type Error = AuthError;

    #[tracing::instrument(skip_all)]
    async fn authenticate(
        &self,
        credentials: Self::Credentials,
    ) -> Result<Option<Self::User>, Self::Error> {
        match credentials {
            Credentials::GitHubOAuth { code } => {
                let token = match self
                    .github_client
                    .exchange_code(oauth2::AuthorizationCode::new(code))
                    .request_async(&self.http_client)
                    .await
                {
                    Ok(token) => token,
                    Err(e) => {
                        tracing::warn!(provider = "github", error = ?e, "login_failure: token exchange failed");
                        return Ok(None);
                    }
                };

                let access_token = token.access_token().secret();
                let github_user = match get_github_user(&self.http_client, access_token).await {
                    Ok(user) => user,
                    Err(e) => {
                        tracing::warn!(provider = "github", error = ?e, "login_failure: user fetch failed");
                        return Ok(None);
                    }
                };
                let provider_user_id = github_user.id.to_string();

                // Look up existing oauth_accounts row
                let existing = sqlx::query_as!(
                    User,
                    r#"SELECT u.id, u.email, u.username, u.avatar_url, u.bio, u.country, u.created_at
                       FROM users u
                       JOIN oauth_accounts oa ON oa.user_id = u.id
                       WHERE oa.provider = 'github' AND oa.provider_user_id = $1"#,
                    provider_user_id,
                )
                .fetch_optional(&self.pool)
                .await?;

                if let Some(user) = existing {
                    tracing::info!(user_id = %user.id, provider = "github", "login_success");
                    return Ok(Some(user));
                }

                // `get_github_user` already filters to primary + verified emails;
                // fall back to a synthetic email when GitHub returns none so
                // account creation doesn't fail. Synthetic emails use a
                // `github_<id>` prefix that can't collide with real addresses.
                let (email, has_verified_email) = match github_user.email {
                    Some(real) => (real, true),
                    None => (format!("github_{}", provider_user_id), false),
                };

                // Only link to an existing user by email if GitHub gave us a
                // verified address — otherwise we'd let anyone hijack accounts
                // by passing a non-verified placeholder.
                let existing_by_email = if has_verified_email {
                    sqlx::query_as!(
                        User,
                        r#"SELECT id, email, username, avatar_url, bio, country, created_at
                           FROM users WHERE email = $1"#,
                        email
                    )
                    .fetch_optional(&self.pool)
                    .await?
                } else {
                    None
                };
                let (user, was_created) = match existing_by_email {
                    Some(user) => (user, false),
                    None => {
                        let user_id = Uuid::new_v4();
                        let new_user = sqlx::query_as!(
                            User,
                            r#"INSERT INTO users (id, email) VALUES ($1, $2)
                               RETURNING id, email, username, avatar_url, bio, country, created_at"#,
                            user_id,
                            email,
                        )
                        .fetch_one(&self.pool)
                        .await?;
                        (new_user, true)
                    }
                };
                sqlx::query!(
                    "INSERT INTO oauth_accounts (user_id, provider, provider_user_id) VALUES ($1, 'github', $2)",
                    user.id,
                    provider_user_id,
                )
                .execute(&self.pool)
                .await?;

                if was_created {
                    tracing::info!(user_id = %user.id, provider = "github", "account_created");
                } else {
                    tracing::info!(user_id = %user.id, provider = "github", "account_linked");
                }
                tracing::info!(user_id = %user.id, provider = "github", "login_success");
                Ok(Some(user))
            }
            Credentials::GoogleOAuth { code, nonce } => {
                let token = match self
                    .google_client
                    .exchange_code(openidconnect::AuthorizationCode::new(code))?
                    .request_async(&self.http_client)
                    .await
                {
                    Ok(token) => token,
                    Err(e) => {
                        tracing::warn!(provider = "google", error = ?e, "login_failure: token exchange failed");
                        return Ok(None);
                    }
                };

                let id_token = match token.extra_fields().id_token() {
                    Some(t) => t,
                    None => {
                        tracing::warn!(provider = "google", "login_failure: no id_token");
                        return Ok(None);
                    }
                };

                let claims = match id_token.claims(
                    &self.google_client.id_token_verifier(),
                    &openidconnect::Nonce::new(nonce),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(provider = "google", error = ?e, "login_failure: claim verification");
                        return Ok(None);
                    }
                };

                let provider_user_id = claims.subject().to_string();
                // Only treat the email as ours to link by if Google says it's verified.
                let verified_email = claims
                    .email()
                    .filter(|_| claims.email_verified().unwrap_or(false))
                    .map(|e| e.to_string());

                let existing = sqlx::query_as!(
                    User,
                    r#"SELECT u.id, u.email, u.username, u.avatar_url, u.bio, u.country, u.created_at
                       FROM users u
                       JOIN oauth_accounts oa ON oa.user_id = u.id
                       WHERE oa.provider = 'google' AND oa.provider_user_id = $1"#,
                    provider_user_id,
                )
                .fetch_optional(&self.pool)
                .await?;

                if let Some(user) = existing {
                    tracing::info!(user_id = %user.id, provider = "google", "login_success");
                    return Ok(Some(user));
                }

                let (email, has_verified_email) = match verified_email {
                    Some(real) => (real, true),
                    None => (format!("google_{}", provider_user_id), false),
                };

                let existing_by_email = if has_verified_email {
                    sqlx::query_as!(
                        User,
                        r#"SELECT id, email, username, avatar_url, bio, country, created_at
                           FROM users WHERE email = $1"#,
                        email
                    )
                    .fetch_optional(&self.pool)
                    .await?
                } else {
                    None
                };
                let (user, was_created) = match existing_by_email {
                    Some(user) => (user, false),
                    None => {
                        let user_id = Uuid::new_v4();
                        let new_user = sqlx::query_as!(
                            User,
                            r#"INSERT INTO users (id, email) VALUES ($1, $2)
                               RETURNING id, email, username, avatar_url, bio, country, created_at"#,
                            user_id,
                            email,
                        )
                        .fetch_one(&self.pool)
                        .await?;
                        (new_user, true)
                    }
                };
                sqlx::query!(
                    "INSERT INTO oauth_accounts (user_id, provider, provider_user_id) VALUES ($1, 'google', $2)",
                    user.id,
                    provider_user_id,
                )
                .execute(&self.pool)
                .await?;

                if was_created {
                    tracing::info!(user_id = %user.id, provider = "google", "account_created");
                } else {
                    tracing::info!(user_id = %user.id, provider = "google", "account_linked");
                }
                tracing::info!(user_id = %user.id, provider = "google", "login_success");
                Ok(Some(user))
            }
        }
    }

    #[tracing::instrument(skip(self), fields(user_id = %id))]
    async fn get_user(&self, id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        Ok(query_as!(
            User,
            r#"SELECT id, email, username, avatar_url, bio, country, created_at
               FROM users WHERE id = $1"#,
            id
        )
        .fetch_optional(&self.pool)
        .await?)
    }
}

// let require = Require::<Backend>::builder()
//     .unauthenticated(RedirectHandler::new().login_url("/login"))
//     .build();
