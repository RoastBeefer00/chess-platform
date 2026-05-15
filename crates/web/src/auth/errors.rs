#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("argon2 error: {0}")]
    Argon2(#[from] argon2::password_hash::Error),
    #[error("task join error: {0}")]
    TaskJoin(#[from] tokio::task::JoinError),
    #[error("reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("no verified primary email found for GitHub user")]
    NoVerifiedPrimaryEmail,
    #[error("an account with this email already exists")]
    EmailAlreadyRegistered,
    #[error("OIDC configuration error: {0}")]
    OidcConfig(#[from] openidconnect::ConfigurationError),
    #[error("Username already taken: {0}")]
    UsernameTaken(String),
}
