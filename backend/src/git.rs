//! Git operations for Spin backend: passive in-process telemetry with bridge forwarding fallback.

use std::path::Path;

use http_body_util::BodyExt;
use openwebide_core::{
    GitBranchInfo, GitCheckoutRequest, GitCheckoutResult, GitCommitRequest, GitCommitResult,
    GitLineStats, GitRepoStatus, GitSyncRequest, GitSyncResult,
};
use spin_sdk::http;

const DEFAULT_BRIDGE_URL: &str = "http://127.0.0.1:3001";

/// Fetch Git repository status.
///
/// Tries the bridge daemon first for active/rich porcelain status and line stats.
/// If the bridge is unreachable, falls back to passive telemetry: reading `.git/HEAD`
/// directly inside the mounted project dir.
pub async fn repo_status(project_full_path: &Path) -> Result<GitRepoStatus, String> {
    // Try bridge first
    let bridge_endpoint = format!("{DEFAULT_BRIDGE_URL}/git/status");
    let payload = serde_json::json!({
        "cwd": project_full_path.to_string_lossy()
    })
    .to_string();

    if let Ok(resp) = http::post(&bridge_endpoint, payload).await
        && resp.status().is_success()
        && let Ok(collected) = resp.into_body().collect().await
        && let Ok(status) = serde_json::from_slice::<GitRepoStatus>(&collected.to_bytes())
    {
        return Ok(status);
    }

    // Passive telemetry fallback: read .git/HEAD
    passive_repo_status(project_full_path)
}

/// In-process passive inspection of `.git/HEAD` and `.git/refs/`.
pub fn passive_repo_status(project_full_path: &Path) -> Result<GitRepoStatus, String> {
    let git_dir = project_full_path.join(".git");
    if !git_dir.exists() {
        return Err("not a git repository".into());
    }

    let head_path = git_dir.join("HEAD");
    let head_content = std::fs::read_to_string(&head_path)
        .map_err(|e| format!("failed to read .git/HEAD: {e}"))?;
    let head_trimmed = head_content.trim();

    let mut branch = String::from("HEAD");
    let mut commit_hash = String::new();

    if let Some(ref_path) = head_trimmed.strip_prefix("ref: ") {
        // e.g. "refs/heads/main"
        branch = ref_path.rsplit('/').next().unwrap_or(ref_path).to_string();
        let branch_ref_file = git_dir.join(ref_path);
        if let Ok(sha) = std::fs::read_to_string(branch_ref_file) {
            commit_hash = sha.trim().to_string();
        }
    } else {
        // Detached HEAD: the content of HEAD is the commit SHA
        commit_hash = head_trimmed.to_string();
    }

    Ok(GitRepoStatus {
        branch,
        commit_hash,
        commit_message: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        is_clean: true,
        line_stats: GitLineStats::default(),
        files: Default::default(),
    })
}

/// Fetch diff from bridge.
pub async fn repo_diff(file_path: Option<&str>) -> Result<String, String> {
    let endpoint = format!("{DEFAULT_BRIDGE_URL}/git/diff");
    let payload = serde_json::json!({
        "path": file_path
    })
    .to_string();

    let resp = http::post(&endpoint, payload)
        .await
        .map_err(|e| format!("bridge diff error: {e}"))?;
    let collected = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| format!("read error: {e}"))?;
    let bytes = collected.to_bytes();

    #[derive(serde::Deserialize)]
    struct DiffOut {
        diff: String,
    }
    serde_json::from_slice::<DiffOut>(&bytes)
        .map(|d| d.diff)
        .map_err(|e| format!("parse error: {e}"))
}

/// Fetch file content at Git HEAD from bridge.
pub async fn repo_file_head(project_full_path: &Path, file_path: &str) -> Result<String, String> {
    let endpoint = format!("{DEFAULT_BRIDGE_URL}/git/show");
    let payload = serde_json::json!({
        "cwd": project_full_path.to_string_lossy(),
        "path": file_path
    })
    .to_string();

    let resp = http::post(&endpoint, payload)
        .await
        .map_err(|e| format!("bridge show error: {e}"))?;
    let collected = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| format!("read error: {e}"))?;
    let bytes = collected.to_bytes();

    #[derive(serde::Deserialize)]
    struct ShowOut {
        content: String,
    }
    serde_json::from_slice::<ShowOut>(&bytes)
        .map(|d| d.content)
        .map_err(|e| format!("parse error: {e}"))
}

/// List branches from bridge.
pub async fn repo_branches() -> Result<Vec<GitBranchInfo>, String> {
    let endpoint = format!("{DEFAULT_BRIDGE_URL}/git/branches");
    let resp = http::post(&endpoint, "{}")
        .await
        .map_err(|e| format!("bridge branches error: {e}"))?;
    let collected = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| format!("read error: {e}"))?;
    serde_json::from_slice::<Vec<GitBranchInfo>>(&collected.to_bytes())
        .map_err(|e| format!("parse error: {e}"))
}

/// Commit changes via bridge.
pub async fn repo_commit(req: &GitCommitRequest) -> Result<GitCommitResult, String> {
    let endpoint = format!("{DEFAULT_BRIDGE_URL}/git/commit");
    let payload = serde_json::to_string(req).map_err(|e| e.to_string())?;
    let resp = http::post(&endpoint, payload)
        .await
        .map_err(|e| format!("bridge commit error: {e}"))?;
    let is_success = resp.status().is_success();
    let collected = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| format!("read error: {e}"))?;
    let bytes = collected.to_bytes();
    if !is_success {
        let text = String::from_utf8_lossy(&bytes);
        return Err(text.into_owned());
    }
    serde_json::from_slice::<GitCommitResult>(&bytes).map_err(|e| format!("parse error: {e}"))
}

/// Checkout branch via bridge.
pub async fn repo_checkout(req: &GitCheckoutRequest) -> Result<GitCheckoutResult, String> {
    let endpoint = format!("{DEFAULT_BRIDGE_URL}/git/checkout");
    let payload = serde_json::to_string(req).map_err(|e| e.to_string())?;
    let resp = http::post(&endpoint, payload)
        .await
        .map_err(|e| format!("bridge checkout error: {e}"))?;
    let is_success = resp.status().is_success();
    let collected = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| format!("read error: {e}"))?;
    let bytes = collected.to_bytes();
    if !is_success {
        let text = String::from_utf8_lossy(&bytes);
        return Err(text.into_owned());
    }
    serde_json::from_slice::<GitCheckoutResult>(&bytes).map_err(|e| format!("parse error: {e}"))
}

/// Sync repo via bridge.
pub async fn repo_sync(req: &GitSyncRequest) -> Result<GitSyncResult, String> {
    let endpoint = format!("{DEFAULT_BRIDGE_URL}/git/sync");
    let payload = serde_json::to_string(req).map_err(|e| e.to_string())?;
    let resp = http::post(&endpoint, payload)
        .await
        .map_err(|e| format!("bridge sync error: {e}"))?;
    let is_success = resp.status().is_success();
    let collected = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| format!("read error: {e}"))?;
    let bytes = collected.to_bytes();
    if !is_success {
        let text = String::from_utf8_lossy(&bytes);
        return Err(text.into_owned());
    }
    serde_json::from_slice::<GitSyncResult>(&bytes).map_err(|e| format!("parse error: {e}"))
}
