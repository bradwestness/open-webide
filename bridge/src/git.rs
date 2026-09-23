//! Native Git execution against the mounted repository.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use openwebide_core::{
    GitBranchInfo, GitCheckoutRequest, GitCheckoutResult, GitCommitRequest, GitCommitResult,
    GitRepoStatus, GitSyncRequest, GitSyncResult, parse_diff_stat, parse_porcelain_v1,
};
use tokio::process::Command;
use tokio::time::timeout;

/// Helper to execute git command with 30s timeout and return stdout/stderr.
async fn exec_git(args: &[&str], cwd: &Path) -> Result<(String, String, bool), String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    cmd.current_dir(cwd);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn git {}: {e}", args.join(" ")))?;

    match timeout(Duration::from_secs(30), child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let success = output.status.success();
            Ok((stdout, stderr, success))
        }
        Ok(Err(e)) => Err(format!("git execution error: {e}")),
        Err(_) => Err("git command timed out after 30s".into()),
    }
}

/// Retrieve full repository status (branch, upstream, ahead/behind, line stats, uncommitted files).
pub async fn get_repo_status(repo_dir: &Path) -> Result<GitRepoStatus, String> {
    // 1. Run git status --porcelain=v1 -b
    let (status_out, err, ok) = exec_git(&["status", "--porcelain=v1", "-b"], repo_dir).await?;
    if !ok {
        return Err(format!("git status failed: {err}"));
    }

    let (files, branch, upstream, ahead, behind) = parse_porcelain_v1(&status_out);
    let is_clean = files.is_empty();

    // 2. Fetch last commit info if available
    let (commit_hash, commit_message) =
        match exec_git(&["log", "-1", "--format=%H%x00%s"], repo_dir).await {
            Ok((log_out, _, true)) if !log_out.is_empty() => {
                if let Some((h, s)) = log_out.split_once('\0') {
                    (h.to_string(), Some(s.to_string()))
                } else {
                    (log_out, None)
                }
            }
            _ => (String::new(), None),
        };

    // 3. Line statistics for uncommitted changes
    let line_stats = match exec_git(&["diff", "--shortstat", "HEAD"], repo_dir).await {
        Ok((diff_out, _, true)) if !diff_out.is_empty() => parse_diff_stat(&diff_out),
        _ => match exec_git(&["diff", "--shortstat"], repo_dir).await {
            Ok((diff_out, _, true)) => parse_diff_stat(&diff_out),
            _ => Default::default(),
        },
    };

    Ok(GitRepoStatus {
        branch,
        commit_hash,
        commit_message,
        upstream,
        ahead,
        behind,
        is_clean,
        line_stats,
        files,
    })
}

/// Retrieve repository or file diff against HEAD or working tree.
pub async fn get_repo_diff(repo_dir: &Path, file_path: Option<&str>) -> Result<String, String> {
    let mut args = vec!["diff"];
    let has_head = exec_git(&["rev-parse", "--verify", "HEAD"], repo_dir)
        .await
        .map(|(_, _, ok)| ok)
        .unwrap_or(false);

    if has_head {
        args.push("HEAD");
    }

    if let Some(path) = file_path {
        args.push("--");
        args.push(path);
    }

    let (diff_out, err, ok) = exec_git(&args, repo_dir).await?;
    if !ok {
        return Err(format!("git diff failed: {err}"));
    }
    Ok(diff_out)
}

/// Retrieve the raw contents of a file at Git HEAD.
pub async fn get_file_at_head(repo_dir: &Path, file_path: &str) -> Result<String, String> {
    let clean_path = file_path.trim_start_matches('/');
    let (out, err, ok) = exec_git(&["show", &format!("HEAD:{clean_path}")], repo_dir).await?;
    if !ok {
        return Err(format!("git show HEAD:{clean_path} failed: {err}"));
    }
    Ok(out)
}

/// List local and remote branches.
pub async fn get_repo_branches(repo_dir: &Path) -> Result<Vec<GitBranchInfo>, String> {
    let (out, err, ok) = exec_git(
        &[
            "branch",
            "-a",
            "--format=%(refname:short)%00%(HEAD)%00%(upstream:short)",
        ],
        repo_dir,
    )
    .await?;

    if !ok {
        return Err(format!("git branch failed: {err}"));
    }

    let mut branches = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split('\0').collect();
        if parts.is_empty() || parts[0].trim().is_empty() {
            continue;
        }

        let name = parts[0].trim().to_string();
        let is_current = parts.get(1).map(|&s| s.trim() == "*").unwrap_or(false);
        let upstream = parts
            .get(2)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let is_remote = name.starts_with("origin/") || name.starts_with("remotes/");

        branches.push(GitBranchInfo {
            name,
            is_current,
            is_remote,
            upstream,
        });
    }

    Ok(branches)
}

/// Create a Git commit on the host.
pub async fn commit_changes(
    repo_dir: &Path,
    req: &GitCommitRequest,
) -> Result<GitCommitResult, String> {
    if req.message.trim().is_empty() {
        return Err("commit message cannot be empty".into());
    }

    // Stage untracked files if requested
    if req.include_untracked {
        let (_, err, ok) = exec_git(&["add", "-A"], repo_dir).await?;
        if !ok {
            return Err(format!("git add -A failed: {err}"));
        }
    }

    // Stage specified paths if provided
    if let Some(paths) = &req.paths
        && !paths.is_empty()
    {
        let mut add_args = vec!["add", "--"];
        for p in paths {
            add_args.push(p.as_str());
        }
        let (_, err, ok) = exec_git(&add_args, repo_dir).await?;
        if !ok {
            return Err(format!("git add failed: {err}"));
        }
    }

    // Execute git commit
    let mut commit_args = vec!["commit", "-m", req.message.as_str()];
    if req.paths.is_none() && !req.include_untracked {
        commit_args.push("-a");
    }

    let (stdout, stderr, ok) = exec_git(&commit_args, repo_dir).await?;
    if !ok {
        let err_msg = if !stderr.is_empty() {
            stderr
        } else {
            stdout.clone()
        };
        return Err(format!("git commit failed: {err_msg}"));
    }

    // Inspect newly created commit
    let (log_out, _, _) = exec_git(&["log", "-1", "--format=%H%x00%s%x00%G?"], repo_dir).await?;
    let mut parts = log_out.split('\0');
    let commit_hash = parts.next().unwrap_or_default().to_string();
    let summary = parts.next().unwrap_or(&req.message).to_string();
    let gpg_flag = parts.next().unwrap_or_default();
    let is_signed = gpg_flag == "G" || gpg_flag == "U";

    Ok(GitCommitResult {
        commit_hash,
        summary,
        pre_commit_output: if stdout.contains("hook") {
            Some(stdout)
        } else {
            None
        },
        is_signed,
    })
}

/// Switch or create a Git branch.
pub async fn checkout_branch(
    repo_dir: &Path,
    req: &GitCheckoutRequest,
) -> Result<GitCheckoutResult, String> {
    let (prev_branch, _, _) = exec_git(&["rev-parse", "--abbrev-ref", "HEAD"], repo_dir).await?;
    let target = req.branch.trim();

    if target.is_empty() {
        return Err("branch name cannot be empty".into());
    }

    let outcome = if req.create_if_missing {
        // Try checkout -b first; if branch exists, fall back to plain checkout
        match exec_git(&["checkout", "-b", target], repo_dir).await {
            Ok((_, _, true)) => Ok(true),
            _ => exec_git(&["checkout", target], repo_dir)
                .await
                .map(|(_, _, ok)| ok),
        }
    } else {
        exec_git(&["checkout", target], repo_dir)
            .await
            .map(|(_, _, ok)| ok)
    };

    match outcome {
        Ok(true) => Ok(GitCheckoutResult {
            branch: target.to_string(),
            previous_branch: if prev_branch.is_empty() {
                None
            } else {
                Some(prev_branch)
            },
            switched: true,
        }),
        Ok(false) => Err(format!("failed to switch to branch '{target}'")),
        Err(e) => Err(e),
    }
}

/// Synchronize with remote (pull, push, or sync).
pub async fn sync_repo(repo_dir: &Path, req: &GitSyncRequest) -> Result<GitSyncResult, String> {
    let (current_branch, _, _) = exec_git(&["rev-parse", "--abbrev-ref", "HEAD"], repo_dir).await?;
    let remote = req.remote.as_deref().unwrap_or("origin");
    let branch = req.branch.as_deref().unwrap_or(&current_branch);

    let mut output = String::new();
    let mut pulled = 0;
    let mut pushed = 0;

    if req.action == "pull" || req.action == "sync" {
        let (pull_out, pull_err, ok) =
            exec_git(&["pull", "--rebase", remote, branch], repo_dir).await?;
        if !ok {
            return Err(format!("git pull failed: {pull_err}"));
        }
        output.push_str(&pull_out);
        if pull_out.contains("Fast-forward") || pull_out.contains("Updating") {
            pulled = 1;
        }
    }

    if req.action == "push" || req.action == "sync" {
        let (push_out, push_err, ok) = exec_git(&["push", remote, branch], repo_dir).await?;
        if !ok {
            return Err(format!("git push failed: {push_err}"));
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&push_out);
        if push_out.contains("->") {
            pushed = 1;
        }
    }

    Ok(GitSyncResult {
        remote: remote.to_string(),
        branch: branch.to_string(),
        pulled_commits: pulled,
        pushed_commits: pushed,
        output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_repo_status_on_current_repo() {
        let repo_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let status = get_repo_status(repo_dir).await;
        assert!(
            status.is_ok(),
            "status on open-webide repo should succeed: {:?}",
            status
        );
        let s = status.unwrap();
        assert!(!s.branch.is_empty());
        assert!(!s.commit_hash.is_empty());
    }

    #[tokio::test]
    async fn test_get_repo_branches_on_current_repo() {
        let repo_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let branches = get_repo_branches(repo_dir).await;
        assert!(branches.is_ok());
        let b = branches.unwrap();
        assert!(!b.is_empty());
        assert!(b.iter().any(|item| item.is_current));
    }
}
