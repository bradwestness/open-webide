//! API error type and JSON response helpers.

use serde_json::json;
use spin_sdk::http::Response;

/// The response type used across the API: a plain `http` response with a
/// `String` body (http-body implements `Body` for `String`).
pub type JsonResp = Response<String>;

pub struct ApiError {
    status: u16,
    message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: 404,
            message: message.into(),
        }
    }

    pub fn not_implemented(message: impl Into<String>) -> Self {
        Self {
            status: 501,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: 500,
            message: message.into(),
        }
    }

    /// Build the JSON error response. A plain method (not an `IntoResponse`
    /// impl) so that `Result<T, ApiError>` handlers can convert it
    /// explicitly.
    pub fn into_response(self) -> JsonResp {
        let body = json!({ "error": self.message }).to_string();
        Response::builder()
            .status(self.status)
            .header("content-type", "application/json")
            .body(body)
            .expect("valid status and headers")
    }
}

impl From<openwebide_storage::StorageError> for ApiError {
    fn from(err: openwebide_storage::StorageError) -> Self {
        match err {
            openwebide_storage::StorageError::NotFound(msg) => Self::not_found(msg),
            other => Self::internal(other.to_string()),
        }
    }
}

impl From<openwebide_llm::ProviderError> for ApiError {
    fn from(err: openwebide_llm::ProviderError) -> Self {
        match err {
            openwebide_llm::ProviderError::NotImplemented(msg) => Self::not_implemented(msg),
            other => Self::internal(other.to_string()),
        }
    }
}
