use hyper::StatusCode;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BridgeError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Execution(String),
    #[error("payload too large")]
    PayloadTooLarge,
    #[error("unsupported media type: application/json required")]
    UnsupportedMediaType,
}

impl BridgeError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Validation(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Execution(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        }
    }
}

impl From<crate::server::http::HttpError> for BridgeError {
    fn from(error: crate::server::http::HttpError) -> Self {
        match error {
            crate::server::http::HttpError::TooLarge => Self::PayloadTooLarge,
            crate::server::http::HttpError::BadRequest(message) => Self::Validation(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_table() {
        for (error, expected) in [
            (BridgeError::Validation("validation".into()), 400),
            (BridgeError::Unauthorized("auth".into()), 401),
            (BridgeError::Forbidden("auth".into()), 403),
            (BridgeError::NotFound("missing".into()), 404),
            (BridgeError::Execution("execution".into()), 500),
            (BridgeError::PayloadTooLarge, 413),
            (BridgeError::UnsupportedMediaType, 415),
        ] {
            assert_eq!(error.status().as_u16(), expected);
        }
    }
}
