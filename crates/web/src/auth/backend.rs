use std::sync::Arc;
use std::time::Instant;

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
use serde::{Deserialize, Serialize};
use sqlx::{query_as, PgPool};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::auth::{get_github_user, AuthError};

type GoogleClient = CoreClient<
    OidcEndpointSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    openidconnect::EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

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
    /// Wrapped in Arc<RwLock<..>> so the JWKS can be swapped at runtime when
    /// Google rotates its signing keys, without restarting the server.
    pub google_client: Arc<RwLock<GoogleClient>>,
    // Fields kept for rebuilding the client on refresh.
    google_client_id: ClientId,
    google_client_secret: ClientSecret,
    google_redirect_uri: RedirectUrl,
    google_issuer: IssuerUrl,
    /// Throttles concurrent refresh attempts — only one per 60 s.
    last_google_refresh: Arc<Mutex<Instant>>,
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
            ClientId::new(std::env::var("GOOGLE_CLIENT_ID").expect("GOOGLE_CLIENT_ID must be set"));
        let google_client_secret = ClientSecret::new(
            std::env::var("GOOGLE_CLIENT_SECRET").expect("GOOGLE_CLIENT_SECRET must be set"),
        );
        let google_redirect_uri = RedirectUrl::new(
            std::env::var("GOOGLE_REDIRECT_URI").expect("GOOGLE_REDIRECT_URI must be set"),
        )
        .expect("Invalid GOOGLE_REDIRECT_URI");
        let google_issuer =
            IssuerUrl::new("https://accounts.google.com".to_string()).expect("Invalid issuer URL");

        let provider_metadata = CoreProviderMetadata::discover_async(
            google_issuer.clone(),
            &http_client,
        )
        .await
        .expect("Failed to discover Google OIDC metadata");
        let google_client = Self::build_google_client(
            provider_metadata,
            google_client_id.clone(),
            google_client_secret.clone(),
            google_redirect_uri.clone(),
        );

        AuthBackend {
            pool,
            http_client,
            github_client,
            google_client: Arc::new(RwLock::new(google_client)),
            google_client_id,
            google_client_secret,
            google_redirect_uri,
            google_issuer,
            // Use a far-past instant so the first reactive refresh is never throttled.
            last_google_refresh: Arc::new(Mutex::new(
                Instant::now() - std::time::Duration::from_secs(3600),
            )),
        }
    }

    /// Build a [`GoogleClient`] from freshly-fetched provider metadata.
    fn build_google_client(
        metadata: CoreProviderMetadata,
        client_id: ClientId,
        client_secret: ClientSecret,
        redirect_uri: RedirectUrl,
    ) -> GoogleClient {
        CoreClient::from_provider_metadata(metadata, client_id, Some(client_secret))
            .set_redirect_uri(redirect_uri)
    }

    /// Re-discover Google's OIDC metadata and swap in a client with fresh JWKS.
    ///
    /// Throttled to at most once per 60 seconds so a burst of failed logins
    /// cannot trigger a discovery storm. Discovery errors are logged and
    /// swallowed — the existing (possibly stale) keys keep serving.
    #[tracing::instrument(skip(self))]
    pub async fn refresh_google_metadata(&self) {
        let mut last = self.last_google_refresh.lock().await;
        if last.elapsed() < std::time::Duration::from_secs(60) {
            tracing::debug!("google_oidc_refresh_skipped: throttled");
            return;
        }

        let metadata = match CoreProviderMetadata::discover_async(
            self.google_issuer.clone(),
            &self.http_client,
        )
        .await
        {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = ?e, "google_oidc_refresh_failed: discovery error");
                return;
            }
        };

        let new_client = Self::build_google_client(
            metadata,
            self.google_client_id.clone(),
            self.google_client_secret.clone(),
            self.google_redirect_uri.clone(),
        );
        *self.google_client.write().await = new_client;
        *last = Instant::now();
        tracing::info!("google_oidc_metadata_refreshed");
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
                // Clone the client out so we don't hold the RwLock guard across
                // any .await points (token exchange, verification, refresh).
                let google_client = self.google_client.read().await.clone();

                let token = match google_client
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

                // First verification attempt using the JWKS already in the cloned client.
                // id_token.claims<'a>(&'a self, ...) -> &'a IdTokenClaims: lifetime tied
                // to id_token, not the verifier, so the owned verifier can be dropped.
                let first_result = {
                    let verifier = google_client.id_token_verifier();
                    id_token.claims(&verifier, &openidconnect::Nonce::new(nonce.clone()))
                };

                let claims = match first_result {
                    Ok(c) => c,
                    Err(e) => {
                        // Likely cause: Google rotated its signing keys since our last
                        // discovery. Refresh the JWKS and retry once with a fresh client.
                        tracing::warn!(
                            provider = "google",
                            error = ?e,
                            "login_failure: verify failed, refreshing OIDC metadata"
                        );
                        self.refresh_google_metadata().await;
                        let fresh_client = self.google_client.read().await.clone();
                        let verifier = fresh_client.id_token_verifier();
                        match id_token.claims(&verifier, &openidconnect::Nonce::new(nonce)) {
                            Ok(c) => c,
                            Err(e) => {
                                tracing::warn!(
                                    provider = "google",
                                    error = ?e,
                                    "login_failure: claim verification failed after refresh"
                                );
                                return Ok(None);
                            }
                        }
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
