//! Outbound bridge client for the Spin backend.
//!
//! Communicates with the companion `openwebide-bridge` daemon via HTTP POST `/exec`
//! to execute workspace commands in remote mode.

use std::future::Future;

use openwebide_agent::BridgeClient;
use openwebide_core::CommandOutcome;

/// Bridge client that delegates process execution to an external bridge daemon
/// via HTTP POST `/exec`.
#[derive(Clone)]
pub struct SpinBridgeClient {
    store: std::sync::Arc<openwebide_storage::Store<crate::state::AppDb>>,
    project_dir: String,
}

impl SpinBridgeClient {
    pub fn for_project(
        store: std::sync::Arc<openwebide_storage::Store<crate::state::AppDb>>,
        dir: String,
    ) -> Self {
        Self {
            store,
            project_dir: dir,
        }
    }
}

impl BridgeClient for SpinBridgeClient {
    fn execute_command(
        &self,
        command: &str,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<CommandOutcome, String>> + Send {
        let payload = serde_json::json!({
            "command": command,
            "timeout_seconds": timeout_seconds,
            "cwd": self.project_dir,
        })
        .to_string();

        let store = self.store.clone();
        async move {
            let (status, body_bytes) = crate::bridge::send(&store, "/exec", payload)
                .await
                .map_err(|e| match e {
                    crate::git::BridgeError::Unreachable(msg) => format!("Failed to connect to bridge daemon: {msg}. Ensure 'openwebide-bridge' is running."),
                    crate::git::BridgeError::Unauthorized => "Bridge error (HTTP 401): bridge secret required".to_string(),
                    crate::git::BridgeError::NoSecret => "Bridge error: no secret configured (set SPIN_VARIABLE_BRIDGE_SECRET)".to_string(),
                    _ => format!("Bridge error: {:?}", e),
                })?;

            if !(200..300).contains(&status) {
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
        let dir = self.project_dir.clone();
        let store = self.store.clone();
        async move {
            crate::git::repo_status(&store, &dir)
                .await
                .map_err(|e| format!("{:?}", e))
        }
    }

    fn git_diff(&self, path: Option<&str>) -> impl Future<Output = Result<String, String>> + Send {
        let path_owned = path.map(|s| s.to_string());
        let dir = self.project_dir.clone();
        let store = self.store.clone();
        async move {
            crate::git::repo_diff(&store, &dir, path_owned.as_deref())
                .await
                .map_err(|e| format!("{:?}", e))
        }
    }

    fn git_commit(
        &self,
        req: &openwebide_core::GitCommitRequest,
    ) -> impl Future<Output = Result<openwebide_core::GitCommitResult, String>> + Send {
        let req_clone = req.clone();
        let dir = self.project_dir.clone();
        let store = self.store.clone();
        async move {
            crate::git::repo_commit(&store, &dir, &req_clone)
                .await
                .map_err(|e| format!("{:?}", e))
        }
    }

    fn git_checkout(
        &self,
        req: &openwebide_core::GitCheckoutRequest,
    ) -> impl Future<Output = Result<openwebide_core::GitCheckoutResult, String>> + Send {
        let req_clone = req.clone();
        let dir = self.project_dir.clone();
        let store = self.store.clone();
        async move {
            crate::git::repo_checkout(&store, &dir, &req_clone)
                .await
                .map_err(|e| format!("{:?}", e))
        }
    }
}
