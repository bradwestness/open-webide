//! API error type and JSON response helpers.

use bytes::Bytes;
use serde_json::json;
use spin_sdk::http::{BoxBody, FullBody, Response, box_body};

/// The response type used across the API: a plain `http` response with a
/// type-erased body, so JSON responses and SSE streams share one type.
pub type JsonResp = Response<BoxBody>;

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
            .body(box_body(FullBody::new(Bytes::from(body))))
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
            openwebide_llm::ProviderError::NoModel => Self::bad_request(err.to_string()),
            other => Self::internal(other.to_string()),
        }
    }
}
