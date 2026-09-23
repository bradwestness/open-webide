//! Git operations for Spin backend: passive in-process telemetry with bridge forwarding fallback.

use std::path::Path;

use http_body_util::BodyExt;
use openwebide_core::{
    GitBranchInfo, GitCheckoutRequest, GitCheckoutResult, GitCommitRequest, GitCommitResult,
    GitLineStats, GitRepoStatus, GitSyncRequest, GitSyncResult,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use spin_sdk::http;

const DEFAULT_BRIDGE_URL: &str = "http://127.0.0.1:3001";

#[derive(Debug)]
pub enum BridgeError {
    Unreachable(String),
    Status(u16, String),
    Parse(String),
}

fn parse_bridge_response<T: DeserializeOwned>(status: u16, body: &[u8]) -> Result<T, BridgeError> {
    if !(200..300).contains(&status) {
        let msg = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|val| {
                val.get("error")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
        return Err(BridgeError::Status(status, msg));
    }
    serde_json::from_slice::<T>(body).map_err(|e| BridgeError::Parse(e.to_string()))
}

fn with_cwd(mut req: serde_json::Value, cwd: &str) -> serde_json::Value {
    if let serde_json::Value::Object(ref mut map) = req {
        map.insert(
            "cwd".to_string(),
            serde_json::Value::String(cwd.to_string()),
        );
    }
    req
}

async fn bridge_post<Req: Serialize, Resp: DeserializeOwned>(
    path: &str,
    cwd: &str,
    req: &Req,
) -> Result<Resp, BridgeError> {
    let endpoint = format!("{DEFAULT_BRIDGE_URL}{path}");
    let val = serde_json::to_value(req).map_err(|e| BridgeError::Parse(e.to_string()))?;
    let val_with_cwd = with_cwd(val, cwd);
    let payload =
        serde_json::to_string(&val_with_cwd).map_err(|e| BridgeError::Parse(e.to_string()))?;

    let resp = http::post(&endpoint, payload)
        .await
        .map_err(|e| BridgeError::Unreachable(e.to_string()))?;

    let status = resp.status().as_u16();
    let collected = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| BridgeError::Unreachable(e.to_string()))?;

    parse_bridge_response(status, &collected.to_bytes())
}

/// Fetch Git repository status.
///
/// Tries the bridge daemon first for active/rich porcelain status and line stats.
/// If the bridge is unreachable, falls back to passive telemetry: reading `.git/HEAD`
/// directly inside the mounted project dir.
pub async fn repo_status(project_dir: &str) -> Result<GitRepoStatus, BridgeError> {
    match bridge_post::<_, GitRepoStatus>("/git/status", project_dir, &serde_json::json!({})).await
    {
        Ok(status) => Ok(status),
        Err(BridgeError::Unreachable(_)) => {
            // Passive telemetry fallback
            let path = Path::new(project_dir);
            match passive_repo_status(path) {
                Ok(mut st) => {
                    st.is_clean = false; // "unknown" status
                    Ok(st)
                }
                Err(_) => {
                    // Return a default "unknown" status if .git doesn't exist or is unreadable
                    Ok(GitRepoStatus {
                        branch: "unknown".into(),
                        commit_hash: "".into(),
                        commit_message: None,
                        upstream: None,
                        ahead: 0,
                        behind: 0,
                        is_clean: false,
                        line_stats: GitLineStats::default(),
                        files: Default::default(),
                    })
                }
            }
        }
        Err(e) => Err(e),
    }
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
pub async fn repo_diff(project_dir: &str, file_path: Option<&str>) -> Result<String, BridgeError> {
    #[derive(serde::Deserialize)]
    struct DiffOut {
        diff: String,
    }
    let res: DiffOut = bridge_post(
        "/git/diff",
        project_dir,
        &serde_json::json!({ "path": file_path }),
    )
    .await?;
    Ok(res.diff)
}

/// Fetch file content at Git HEAD from bridge.
pub async fn repo_file_head(project_dir: &str, file_path: &str) -> Result<String, BridgeError> {
    #[derive(serde::Deserialize)]
    struct ShowOut {
        content: String,
    }
    let res: ShowOut = bridge_post(
        "/git/show",
        project_dir,
        &serde_json::json!({ "path": file_path }),
    )
    .await?;
    Ok(res.content)
}

/// List branches from bridge.
pub async fn repo_branches(project_dir: &str) -> Result<Vec<GitBranchInfo>, BridgeError> {
    bridge_post("/git/branches", project_dir, &serde_json::json!({})).await
}

/// Commit changes via bridge.
pub async fn repo_commit(
    project_dir: &str,
    req: &GitCommitRequest,
) -> Result<GitCommitResult, BridgeError> {
    bridge_post("/git/commit", project_dir, req).await
}

/// Checkout branch via bridge.
pub async fn repo_checkout(
    project_dir: &str,
    req: &GitCheckoutRequest,
) -> Result<GitCheckoutResult, BridgeError> {
    bridge_post("/git/checkout", project_dir, req).await
}

/// Sync repo via bridge.
pub async fn repo_sync(
    project_dir: &str,
    req: &GitSyncRequest,
) -> Result<GitSyncResult, BridgeError> {
    bridge_post("/git/sync", project_dir, req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bridge_response_ok() {
        let json = b"{\"hello\": \"world\"}";
        let res: serde_json::Value = parse_bridge_response(200, json).unwrap();
        assert_eq!(res["hello"], "world");
    }

    #[test]
    fn test_parse_bridge_response_status_500() {
        let json = b"{\"error\": \"boom\"}";
        let res: Result<serde_json::Value, _> = parse_bridge_response(500, json);
        match res {
            Err(BridgeError::Status(500, msg)) => assert_eq!(msg, "boom"),
            _ => panic!("unexpected result"),
        }
    }

    #[test]
    fn test_parse_bridge_response_parse_error() {
        let json = b"{\"error\": \"not the expected schema\"}";
        #[derive(serde::Deserialize)]
        struct DiffOut {
            diff: String,
        }
        let res: Result<DiffOut, _> = parse_bridge_response(200, json);
        match res {
            Err(BridgeError::Parse(_)) => (),
            _ => panic!("unexpected result"),
        }
    }

    #[test]
    fn test_with_cwd() {
        let val = serde_json::json!({ "path": "test" });
        let val = with_cwd(val, "/my/cwd");
        assert_eq!(val["cwd"], "/my/cwd");
        assert_eq!(val["path"], "test");

        // Overwrites
        let val = serde_json::json!({ "cwd": "/old/cwd" });
        let val = with_cwd(val, "/my/cwd");
        assert_eq!(val["cwd"], "/my/cwd");
    }
}
