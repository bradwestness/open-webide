use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("not implemented: {0}")]
    NotImplemented(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("failed to parse provider response: {0}")]
    Parse(String),
}
