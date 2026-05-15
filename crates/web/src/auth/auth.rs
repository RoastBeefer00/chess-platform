use argon2::{
    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
    Argon2, PasswordHash, PasswordVerifier,
};
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
    password_hash: Option<String>,
    created_at: time::OffsetDateTime,
}

impl AuthUser for User {
    type Id = Uuid;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn session_auth_hash(&self) -> &[u8] {
        self.password_hash
            .as_ref()
            .map(|s| s.as_bytes())
            .unwrap_or_default()
    }
}

pub enum Credentials {
    Password { email: String, password: String },
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

impl AuthBackend {
    pub async fn register(&self, email: String, password: String) -> Result<User, AuthError> {
        // Reject duplicate emails up front. We intentionally do NOT link a
        // password registration to an existing OAuth-created account: the
        // email isn't verified at this point, so linking would let anyone
        // hijack an existing account by registering with its email.
        if sqlx::query_scalar!("SELECT 1 FROM users WHERE email = $1", email)
            .fetch_optional(&self.pool)
            .await?
            .is_some()
        {
            return Err(AuthError::EmailAlreadyRegistered);
        }

        let salt = SaltString::generate(&mut OsRng);
        let hash = tokio::task::spawn_blocking(move || {
            Argon2::default()
                .hash_password(password.as_bytes(), &salt)
                .map(|h| h.to_string())
        })
        .await??;

        let id = Uuid::new_v4();
        Ok(sqlx::query_as!(
            User,
            "INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3) RETURNING *",
            id,
            email,
            hash,
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn is_username_available(&self, username: String) -> Result<bool, AuthError> {
        Ok(
            sqlx::query_scalar!("SELECT 1 FROM users WHERE username = $1", username)
                .fetch_optional(&self.pool)
                .await?
                .is_none(),
        )
    }

    pub async fn set_username(&self, user_id: Uuid, username: String) -> Result<(), AuthError> {
        if self.is_username_available(username.clone()).await? {
            sqlx::query!(
                "UPDATE users SET username = $1 WHERE id = $2",
                username,
                user_id
            )
            .execute(&self.pool)
            .await?;
            Ok(())
        } else {
            Err(AuthError::UsernameTaken(username))
        }
    }
}

impl AuthnBackend for AuthBackend {
    type User = User;
    type Credentials = Credentials;
    type Error = AuthError;

    async fn authenticate(
        &self,
        credentials: Self::Credentials,
    ) -> Result<Option<Self::User>, Self::Error> {
        match credentials {
            Credentials::Password { email, password } => {
                let user = match query_as!(User, "SELECT * FROM users WHERE email = $1", email)
                    .fetch_optional(&self.pool)
                    .await?
                {
                    Some(record) => record,
                    None => return Ok(None),
                };

                let Some(hash_str) = user.password_hash.clone() else {
                    return Ok(None);
                };

                let verified = tokio::task::spawn_blocking(move || {
                    let Ok(parsed_hash) = PasswordHash::new(&hash_str) else {
                        return false;
                    };
                    Argon2::default()
                        .verify_password(password.as_bytes(), &parsed_hash)
                        .is_ok()
                })
                .await?;

                if verified {
                    Ok(Some(user))
                } else {
                    Ok(None)
                }
            }
            Credentials::GitHubOAuth { code } => {
                let token = match self
                    .github_client
                    .exchange_code(oauth2::AuthorizationCode::new(code))
                    .request_async(&self.http_client)
                    .await
                {
                    Ok(token) => token,
                    Err(e) => {
                        tracing::error!("github token exchange failed: {e:?}");
                        return Ok(None);
                    }
                };

                let access_token = token.access_token().secret();
                let github_user = match get_github_user(&self.http_client, access_token).await {
                    Ok(user) => user,
                    Err(e) => {
                        tracing::error!("github user fetch failed: {e:?}");
                        return Ok(None);
                    }
                };
                let provider_user_id = github_user.id.to_string();

                // Look up existing oauth_accounts row
                let existing = sqlx::query_as!(
                    User,
                    r#"SELECT u.* FROM users u
                       JOIN oauth_accounts oa ON oa.user_id = u.id
                       WHERE oa.provider = 'github' AND oa.provider_user_id = $1"#,
                    provider_user_id,
                )
                .fetch_optional(&self.pool)
                .await?;

                if let Some(user) = existing {
                    return Ok(Some(user));
                }

                let email = github_user
                    .email
                    .unwrap_or_else(|| format!("github_{}", provider_user_id));

                // Link to an existing user with this email (e.g. one created via Google OAuth)
                // or create a new user.
                let existing_by_email =
                    sqlx::query_as!(User, "SELECT * FROM users WHERE email = $1", email)
                        .fetch_optional(&self.pool)
                        .await?;
                let user = match existing_by_email {
                    Some(user) => user,
                    None => {
                        let user_id = Uuid::new_v4();
                        sqlx::query_as!(
                            User,
                            "INSERT INTO users (id, email, password_hash) VALUES ($1, $2, NULL) RETURNING *",
                            user_id,
                            email,
                        )
                        .fetch_one(&self.pool)
                        .await?
                    }
                };
                sqlx::query!(
                    "INSERT INTO oauth_accounts (user_id, provider, provider_user_id) VALUES ($1, 'github', $2)",
                    user.id,
                    provider_user_id,
                )
                .execute(&self.pool)
                .await?;

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
                    Err(_) => return Ok(None),
                };

                let id_token = match token.extra_fields().id_token() {
                    Some(t) => t,
                    None => return Ok(None),
                };

                let claims = match id_token.claims(
                    &self.google_client.id_token_verifier(),
                    &openidconnect::Nonce::new(nonce),
                ) {
                    Ok(c) => c,
                    Err(_) => return Ok(None),
                };

                let provider_user_id = claims.subject().to_string();
                let email = claims.email().map(|e| e.to_string());

                let existing = sqlx::query_as!(
                    User,
                    r#"SELECT u.* FROM users u
                       JOIN oauth_accounts oa ON oa.user_id = u.id
                       WHERE oa.provider = 'google' AND oa.provider_user_id = $1"#,
                    provider_user_id,
                )
                .fetch_optional(&self.pool)
                .await?;

                if let Some(user) = existing {
                    return Ok(Some(user));
                }

                let email = email.unwrap_or_else(|| format!("google_{}", provider_user_id));

                // Link to an existing user with this email (e.g. one created via GitHub OAuth)
                // or create a new user.
                let existing_by_email =
                    sqlx::query_as!(User, "SELECT * FROM users WHERE email = $1", email)
                        .fetch_optional(&self.pool)
                        .await?;
                let user = match existing_by_email {
                    Some(user) => user,
                    None => {
                        let user_id = Uuid::new_v4();
                        sqlx::query_as!(
                            User,
                            "INSERT INTO users (id, email, password_hash) VALUES ($1, $2, NULL) RETURNING *",
                            user_id,
                            email,
                        )
                        .fetch_one(&self.pool)
                        .await?
                    }
                };
                sqlx::query!(
                    "INSERT INTO oauth_accounts (user_id, provider, provider_user_id) VALUES ($1, 'google', $2)",
                    user.id,
                    provider_user_id,
                )
                .execute(&self.pool)
                .await?;

                Ok(Some(user))
            }
        }
    }

    async fn get_user(&self, id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        Ok(query_as!(User, "SELECT * FROM users WHERE id = $1", id)
            .fetch_optional(&self.pool)
            .await?)
    }
}

// let require = Require::<Backend>::builder()
//     .unauthenticated(RedirectHandler::new().login_url("/login"))
//     .build();
