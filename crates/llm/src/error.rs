use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("not implemented: {0}")]
    NotImplemented(String),
    #[error("no model selected; set a model on the connection or in the request")]
    NoModel,
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("failed to parse provider response: {0}")]
    Parse(String),
}
