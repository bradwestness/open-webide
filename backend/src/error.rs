//! API error type and JSON response helpers.

use bytes::Bytes;
use serde_json::json;
use spin_sdk::http::{BoxBody, FullBody, Response, box_body};

/// The response type used across the API: a plain `http` response with a
/// type-erased body, so JSON responses and SSE streams share one type.
pub type JsonResp = Response<BoxBody>;

#[derive(Debug)]
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

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: 401,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: 403,
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

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: 409,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: 500,
            message: message.into(),
        }
    }

    pub fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: 502,
            message: message.into(),
        }
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self {
            status: 413,
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
            openwebide_storage::StorageError::Conflict(msg) => Self::conflict(msg),
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

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        let msg = err.to_string();
        if msg.contains("already exists in workspace") {
            Self::conflict(msg)
        } else if msg.contains("escapes the workspace root") || msg.contains("is reserved") {
            Self::bad_request(msg)
        } else if msg.contains("filesystem error: NoEntry")
            || msg.contains("not found in workspace")
        {
            Self::not_found(msg)
        } else if msg.contains("not valid UTF-8") || msg.contains("too large to read") {
            Self::bad_request(msg)
        } else {
            Self::internal(msg)
        }
    }
}

impl From<crate::git::BridgeError> for ApiError {
    fn from(err: crate::git::BridgeError) -> Self {
        match err {
            crate::git::BridgeError::Unreachable(e) => Self::bad_gateway(format!(
                "bridge daemon unreachable: {e} please start openwebide-bridge"
            )),
            crate::git::BridgeError::Status(status, msg) => {
                if (400..500).contains(&status) {
                    Self::bad_request(msg)
                } else {
                    Self::bad_gateway(msg)
                }
            }
            crate::git::BridgeError::Parse(e) => {
                Self::bad_gateway(format!("failed to parse bridge response: {e}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_exists_in_workspace_maps_to_409() {
        let err: ApiError = anyhow::anyhow!("already exists in workspace: src/main.rs").into();
        let resp = err.into_response();
        assert_eq!(resp.status().as_u16(), 409);
    }

    #[test]
    fn payload_too_large_maps_to_413() {
        let err = ApiError::payload_too_large("request body exceeds 65536 bytes");
        assert_eq!(err.status, 413);
        assert_eq!(err.message, "request body exceeds 65536 bytes");
        assert_eq!(err.into_response().status().as_u16(), 413);
    }

    #[test]
    fn test_bridge_error_to_api_error() {
        let err: ApiError = crate::git::BridgeError::Unreachable("timeout".into()).into();
        assert_eq!(err.status, 502);
        assert!(err.message.contains("unreachable"));

        let err: ApiError = crate::git::BridgeError::Status(400, "bad args".into()).into();
        assert_eq!(err.status, 400);
        assert_eq!(err.message, "bad args");

        let err: ApiError = crate::git::BridgeError::Status(404, "not found".into()).into();
        assert_eq!(err.status, 400);

        let err: ApiError = crate::git::BridgeError::Status(500, "boom".into()).into();
        assert_eq!(err.status, 502);
        assert_eq!(err.message, "boom");

        let err: ApiError = crate::git::BridgeError::Parse("bad json".into()).into();
        assert_eq!(err.status, 502);
    }
}
