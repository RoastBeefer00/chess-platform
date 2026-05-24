#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("no verified primary email found for GitHub user")]
    NoVerifiedPrimaryEmail,
    #[error("OIDC configuration error: {0}")]
    OidcConfig(#[from] openidconnect::ConfigurationError),
    #[error("Username already taken: {0}")]
    UsernameTaken(String),
}
