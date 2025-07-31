use thiserror::Error;

/// Custom error types for XGA module
#[derive(Debug, Error)]
pub enum Error {
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),


    #[error("Request timeout")]
    Timeout,

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("System time error: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("Signature verification failed")]
    SignatureVerification,
}

/// Convenience type alias for Results using our Error type
pub type Result<T> = std::result::Result<T, Error>;
