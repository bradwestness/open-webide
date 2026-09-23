//! Outbound bridge client for the Spin backend.
//!
//! Communicates with the companion `openwebide-bridge` daemon via HTTP POST `/exec`
//! to execute workspace commands in remote mode.

use std::future::Future;

use http_body_util::BodyExt;
use openwebide_agent::BridgeClient;
use openwebide_core::CommandOutcome;
use spin_sdk::http;

/// Bridge client that delegates process execution to an external bridge daemon
/// via HTTP POST `/exec`.
#[derive(Debug, Clone)]
pub struct SpinBridgeClient {
    endpoint: String,
}

impl SpinBridgeClient {
    #[allow(dead_code)]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }
}

impl Default for SpinBridgeClient {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:3001/exec".to_string(),
        }
    }
}

impl BridgeClient for SpinBridgeClient {
    fn execute_command(
        &self,
        command: &str,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<CommandOutcome, String>> + Send {
        let endpoint = self.endpoint.clone();
        let payload = serde_json::json!({
            "command": command,
            "timeout_seconds": timeout_seconds,
        })
        .to_string();

        async move {
            let response = http::post(&endpoint, payload).await.map_err(|e| {
                format!(
                    "Failed to connect to bridge daemon at {endpoint}: {e}. Ensure 'openwebide-bridge' is running."
                )
            })?;

            let status = response.status();
            let collected = response
                .into_body()
                .collect()
                .await
                .map_err(|e| format!("Failed to read bridge response body: {e}"))?;
            let body_bytes = collected.to_bytes();

            if !status.is_success() {
                let err_text = String::from_utf8_lossy(&body_bytes);
                return Err(format!("Bridge error (HTTP {status}): {err_text}"));
            }

            serde_json::from_slice::<CommandOutcome>(&body_bytes)
                .map_err(|e| format!("Failed to parse bridge command outcome JSON: {e}"))
        }
    }

    #[allow(clippy::manual_async_fn)] // matches the explicit `impl Future + Send` style of the sibling trait methods below
    fn git_status(
        &self,
    ) -> impl Future<Output = Result<openwebide_core::GitRepoStatus, String>> + Send {
        async { crate::git::repo_status(std::path::Path::new("")).await }
    }

    fn git_diff(&self, path: Option<&str>) -> impl Future<Output = Result<String, String>> + Send {
        let path_owned = path.map(|s| s.to_string());
        async move { crate::git::repo_diff(path_owned.as_deref()).await }
    }

    fn git_commit(
        &self,
        req: &openwebide_core::GitCommitRequest,
    ) -> impl Future<Output = Result<openwebide_core::GitCommitResult, String>> + Send {
        let req_clone = req.clone();
        async move { crate::git::repo_commit(&req_clone).await }
    }

    fn git_checkout(
        &self,
        req: &openwebide_core::GitCheckoutRequest,
    ) -> impl Future<Output = Result<openwebide_core::GitCheckoutResult, String>> + Send {
        let req_clone = req.clone();
        async move { crate::git::repo_checkout(&req_clone).await }
    }
}
