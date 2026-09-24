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

/// Git execution and validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    Validation(String),
    Execution(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(msg) | Self::Execution(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for GitError {}

/// Helper to execute git command with 30s timeout and return untrimmed stdout/stderr.
async fn exec_git(args: &[&str], cwd: &Path) -> Result<(String, String, bool), GitError> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    cmd.current_dir(cwd);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }

    let child = cmd
        .spawn()
        .map_err(|e| GitError::Execution(format!("failed to spawn git {}: {e}", args.join(" "))))?;
    #[cfg(unix)]
    let pid = child.id().map(|id| id as i32);

    match timeout(Duration::from_secs(30), child.wait_with_output()).await {
        Ok(Ok(output)) => {
            if output.stdout.len() > 16 * 1024 * 1024 {
                return Err(GitError::Execution("git output exceeds 16 MiB".into()));
            }
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let success = output.status.success();
            Ok((stdout, stderr, success))
        }
        Ok(Err(e)) => Err(GitError::Execution(format!("git execution error: {e}"))),
        Err(_) => {
            #[cfg(unix)]
            if let Some(pgid) = pid {
                crate::proc::terminate_group(pgid, Duration::from_secs(2)).await;
            }
            Err(GitError::Execution(
                "git command timed out after 30s".into(),
            ))
        }
    }
}

/// Validate branch name: reject empty, leading '-', or invalid format according to git check-ref-format.
pub async fn validate_branch(name: &str) -> Result<(), GitError> {
    if name.is_empty() {
        return Err(GitError::Validation("branch name cannot be empty".into()));
    }
    if name.starts_with('-') {
        return Err(GitError::Validation(format!(
            "invalid branch name '{name}': leading '-' not allowed"
        )));
    }

    let mut cmd = Command::new("git");
    cmd.args(["check-ref-format", "--branch", name]);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    cmd.env("GIT_TERMINAL_PROMPT", "0");

    let output = match timeout(Duration::from_secs(5), cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return Err(GitError::Execution(format!(
                "failed to run git check-ref-format: {e}"
            )));
        }
        Err(_) => return Err(GitError::Execution("git check-ref-format timed out".into())),
    };

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let msg = err.trim();
        return Err(GitError::Validation(if msg.is_empty() {
            format!("invalid branch name '{name}'")
        } else {
            format!("invalid branch name '{name}': {msg}")
        }));
    }

    Ok(())
}

/// Validate remote name: reject empty, leading '-', or non-members of `git remote`.
pub async fn validate_remote(repo: &Path, name: &str) -> Result<(), GitError> {
    if name.is_empty() {
        return Err(GitError::Validation("remote name cannot be empty".into()));
    }
    if name.starts_with('-') {
        return Err(GitError::Validation(format!(
            "invalid remote name '{name}': leading '-' not allowed"
        )));
    }

    let (out, err, ok) = exec_git(&["remote"], repo).await?;
    if !ok {
        return Err(GitError::Execution(format!("git remote failed: {err}")));
    }

    let is_member = out.lines().any(|l| l.trim() == name);
    if !is_member {
        return Err(GitError::Validation(format!("unknown remote '{name}'")));
    }

    Ok(())
}

/// Retrieve full repository status (branch, upstream, ahead/behind, line stats, uncommitted files).
pub async fn get_repo_status(repo_dir: &Path) -> Result<GitRepoStatus, GitError> {
    // 1. Run git status --porcelain=v1 -b -z (NUL-separated, unquoted paths)
    let (status_out, err, ok) =
        exec_git(&["status", "--porcelain=v1", "-b", "-z"], repo_dir).await?;
    if !ok {
        return Err(GitError::Execution(format!("git status failed: {err}")));
    }

    let (files, branch, upstream, ahead, behind) = parse_porcelain_v1(&status_out);
    let is_clean = files.is_empty();

    // 2. Fetch last commit info if available
    let (commit_hash, commit_message) =
        match exec_git(&["log", "-1", "--format=%H%x00%s"], repo_dir).await {
            Ok((log_out, _, true)) => {
                let trimmed = log_out.trim_end_matches('\n');
                if !trimmed.is_empty() {
                    if let Some((h, s)) = trimmed.split_once('\0') {
                        (h.trim().to_string(), Some(s.to_string()))
                    } else {
                        (trimmed.trim().to_string(), None)
                    }
                } else {
                    (String::new(), None)
                }
            }
            _ => (String::new(), None),
        };

    // 3. Line statistics for uncommitted changes
    let line_stats = match exec_git(&["diff", "--shortstat", "HEAD"], repo_dir).await {
        Ok((diff_out, _, true)) if !diff_out.trim().is_empty() => parse_diff_stat(diff_out.trim()),
        _ => match exec_git(&["diff", "--shortstat"], repo_dir).await {
            Ok((diff_out, _, true)) => parse_diff_stat(diff_out.trim()),
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
pub async fn get_repo_diff(repo_dir: &Path, file_path: Option<&str>) -> Result<String, GitError> {
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
        return Err(GitError::Execution(format!("git diff failed: {err}")));
    }
    Ok(diff_out)
}

/// Retrieve the raw contents of a file at Git HEAD.
pub async fn get_file_at_head(repo_dir: &Path, file_path: &str) -> Result<String, GitError> {
    let clean_path = file_path.trim_start_matches('/');
    let (out, err, ok) = exec_git(&["show", &format!("HEAD:{clean_path}")], repo_dir).await?;
    if !ok {
        return Err(GitError::Execution(format!(
            "git show HEAD:{clean_path} failed: {err}"
        )));
    }
    Ok(out)
}

/// List local and remote branches.
pub async fn get_repo_branches(repo_dir: &Path) -> Result<Vec<GitBranchInfo>, GitError> {
    let (out, err, ok) = exec_git(
        &[
            "branch",
            "-a",
            "--format=%(refname)%00%(HEAD)%00%(upstream:short)%00%(symref)",
        ],
        repo_dir,
    )
    .await?;

    if !ok {
        return Err(GitError::Execution(format!("git branch failed: {err}")));
    }

    let mut branches = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.split('\0').collect();
        if parts.is_empty() || parts[0].trim().is_empty() {
            continue;
        }

        let symref = parts.get(3).map(|s| s.trim()).unwrap_or("");
        if !symref.is_empty() {
            continue;
        }

        let refname = parts[0].trim();
        let is_remote = refname.starts_with("refs/remotes/");
        let name = if let Some(stripped) = refname.strip_prefix("refs/heads/") {
            stripped.to_string()
        } else if let Some(stripped) = refname.strip_prefix("refs/remotes/") {
            stripped.to_string()
        } else {
            refname.to_string()
        };

        let is_current = parts.get(1).map(|&s| s.trim() == "*").unwrap_or(false);
        let upstream = parts
            .get(2)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

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
) -> Result<GitCommitResult, GitError> {
    if req.message.trim().is_empty() {
        return Err(GitError::Validation(
            "commit message cannot be empty".into(),
        ));
    }

    let has_paths = req.paths.as_ref().is_some_and(|paths| !paths.is_empty());

    let (stdout, stderr, ok) = if has_paths {
        let paths = req.paths.as_ref().unwrap();
        let mut add_args = vec!["add", "--"];
        for p in paths {
            add_args.push(p.as_str());
        }
        let (_, err, ok) = exec_git(&add_args, repo_dir).await?;
        if !ok {
            return Err(GitError::Execution(format!("git add failed: {err}")));
        }

        let mut commit_args = vec!["commit", "-m", req.message.as_str(), "--"];
        for p in paths {
            commit_args.push(p.as_str());
        }
        exec_git(&commit_args, repo_dir).await?
    } else {
        if req.include_untracked {
            let (_, err, ok) = exec_git(&["add", "-A"], repo_dir).await?;
            if !ok {
                return Err(GitError::Execution(format!("git add -A failed: {err}")));
            }
        }

        let mut commit_args = vec!["commit", "-m", req.message.as_str()];
        if !req.include_untracked {
            commit_args.push("-a");
        }
        exec_git(&commit_args, repo_dir).await?
    };

    if !ok {
        let err_msg = if !stderr.is_empty() {
            stderr
        } else {
            stdout.clone()
        };
        return Err(GitError::Execution(format!("git commit failed: {err_msg}")));
    }

    let (log_out, _, _) = exec_git(&["log", "-1", "--format=%H%x00%s%x00%G?"], repo_dir).await?;
    let mut parts = log_out.split('\0');
    let commit_hash = parts.next().unwrap_or_default().trim().to_string();
    let summary = parts.next().unwrap_or(&req.message).to_string();
    let gpg_flag = parts.next().unwrap_or_default().trim();
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
) -> Result<GitCheckoutResult, GitError> {
    let target = req.branch.trim();
    validate_branch(target).await?;

    let (prev_branch_raw, _, _) =
        exec_git(&["rev-parse", "--abbrev-ref", "HEAD"], repo_dir).await?;
    let prev_branch = prev_branch_raw.trim();
    let previous_branch = if prev_branch.is_empty() || prev_branch == "HEAD" {
        None
    } else {
        Some(prev_branch.to_string())
    };

    if req.create_if_missing {
        let (_stdout, stderr, ok) = exec_git(&["switch", "-c", target], repo_dir).await?;
        if !ok {
            let (_, _, branch_exists) = exec_git(
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{target}"),
                ],
                repo_dir,
            )
            .await?;

            if branch_exists {
                let (_sw_stdout, sw_stderr, sw_ok) =
                    exec_git(&["switch", target], repo_dir).await?;
                if !sw_ok {
                    return Err(GitError::Execution(format!(
                        "git switch failed: {}",
                        sw_stderr.trim()
                    )));
                }
            } else {
                return Err(GitError::Execution(format!(
                    "git switch failed: {}",
                    stderr.trim()
                )));
            }
        }
    } else {
        let (_stdout, stderr, ok) = exec_git(&["switch", target], repo_dir).await?;
        if !ok {
            return Err(GitError::Execution(format!(
                "git switch failed: {}",
                stderr.trim()
            )));
        }
    }

    Ok(GitCheckoutResult {
        branch: target.to_string(),
        previous_branch,
        switched: true,
    })
}

/// Synchronize with remote (pull, push, or sync).
pub async fn sync_repo(repo_dir: &Path, req: &GitSyncRequest) -> Result<GitSyncResult, GitError> {
    if req.action != "pull" && req.action != "push" && req.action != "sync" {
        return Err(GitError::Validation(format!(
            "invalid sync action '{}': must be 'pull', 'push', or 'sync'",
            req.action
        )));
    }

    let (current_branch_raw, _, _) =
        exec_git(&["rev-parse", "--abbrev-ref", "HEAD"], repo_dir).await?;
    let current_branch = current_branch_raw.trim();
    let remote = req.remote.as_deref().unwrap_or("origin");
    let branch = req.branch.as_deref().unwrap_or(current_branch);

    validate_remote(repo_dir, remote).await?;
    validate_branch(branch).await?;

    let mut output = String::new();
    let mut pulled_commits = 0;
    let mut pushed_commits = 0;

    let append_output = |target: &mut String, out: &str, err: &str| {
        let combined = match (out.trim().is_empty(), err.trim().is_empty()) {
            (false, false) => format!("{}\n{}", out.trim_end(), err.trim_end()),
            (false, true) => out.trim_end().to_string(),
            (true, false) => err.trim_end().to_string(),
            (true, true) => String::new(),
        };
        if !combined.is_empty() {
            if !target.is_empty() {
                target.push('\n');
            }
            target.push_str(&combined);
        }
    };

    if req.action == "pull" || req.action == "sync" {
        let (old_head_raw, _, _) = exec_git(&["rev-parse", "HEAD"], repo_dir).await?;
        let old_head = old_head_raw.trim().to_string();

        let (pull_out, pull_err, ok) =
            exec_git(&["pull", "--rebase", "--", remote, branch], repo_dir).await?;
        append_output(&mut output, &pull_out, &pull_err);
        if !ok {
            return Err(GitError::Execution(format!(
                "git pull failed: {}",
                pull_err.trim()
            )));
        }

        let (new_head_raw, _, _) = exec_git(&["rev-parse", "HEAD"], repo_dir).await?;
        let new_head = new_head_raw.trim().to_string();

        if !old_head.is_empty() && !new_head.is_empty() && old_head != new_head {
            let (count_out, _, count_ok) = exec_git(
                &["rev-list", "--count", &format!("{old_head}..{new_head}")],
                repo_dir,
            )
            .await?;
            if count_ok {
                pulled_commits = count_out.trim().parse::<usize>().unwrap_or(0);
            }
        } else if old_head.is_empty() && !new_head.is_empty() {
            let (count_out, _, count_ok) =
                exec_git(&["rev-list", "--count", &new_head], repo_dir).await?;
            if count_ok {
                pulled_commits = count_out.trim().parse::<usize>().unwrap_or(0);
            }
        }
    }

    if req.action == "push" || req.action == "sync" {
        let remote_ref = format!("{remote}/{branch}");
        let sha_before = match exec_git(&["rev-parse", "--verify", &remote_ref], repo_dir).await {
            Ok((sha_out, _, true)) if !sha_out.trim().is_empty() => {
                Some(sha_out.trim().to_string())
            }
            _ => None,
        };

        let (push_out, push_err, ok) = exec_git(&["push", "--", remote, branch], repo_dir).await?;
        append_output(&mut output, &push_out, &push_err);
        if !ok {
            return Err(GitError::Execution(format!(
                "git push failed: {}",
                push_err.trim()
            )));
        }

        let rev_list_arg = match &sha_before {
            Some(sha) => format!("{sha}..HEAD"),
            None => "HEAD".to_string(),
        };
        let (count_out, _, count_ok) =
            exec_git(&["rev-list", "--count", &rev_list_arg], repo_dir).await?;
        if count_ok {
            pushed_commits = count_out.trim().parse::<usize>().unwrap_or(0);
        }
    }

    Ok(GitSyncResult {
        remote: remote.to_string(),
        branch: branch.to_string(),
        pulled_commits,
        pushed_commits,
        output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let count = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
            let unique = format!(
                "openwebide-git-test-{}-{}-{}",
                std::process::id(),
                count,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&path).expect("failed to create test temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    async fn create_test_repo() -> TestDir {
        let td = TestDir::new();
        let mut cmd = Command::new("git");
        cmd.args(["init", "-b", "main"]);
        cmd.current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        let mut cmd = Command::new("git");
        cmd.args(["config", "user.name", "Test User"]);
        cmd.current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        let mut cmd = Command::new("git");
        cmd.args(["config", "user.email", "test@example.com"]);
        cmd.current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        td
    }

    #[tokio::test]
    async fn show_preserves_leading_and_trailing_whitespace() {
        let td = create_test_repo().await;
        let file_path = td.path.join("spaced.txt");
        let exact_content = "\n\n  indented\n\n";
        std::fs::write(&file_path, exact_content).unwrap();

        let mut cmd = Command::new("git");
        cmd.args(["add", "spaced.txt"]).current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        let mut cmd = Command::new("git");
        cmd.args(["commit", "-m", "spaced"]).current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        let content = get_file_at_head(&td.path, "spaced.txt").await.unwrap();
        assert_eq!(content, exact_content);
    }

    #[tokio::test]
    async fn checkout_dot_rejected_and_worktree_untouched() {
        let td = create_test_repo().await;
        let work_path = td.path.join("work.txt");
        std::fs::write(&work_path, "important uncommitted work").unwrap();

        let req = GitCheckoutRequest {
            branch: ".".into(),
            create_if_missing: false,
        };
        let res = checkout_branch(&td.path, &req).await;
        assert!(matches!(res, Err(GitError::Validation(_))));

        assert_eq!(
            std::fs::read_to_string(&work_path).unwrap(),
            "important uncommitted work"
        );
    }

    #[tokio::test]
    async fn checkout_existing_branch_with_create_if_missing_switches() {
        let td = create_test_repo().await;
        let file_path = td.path.join("init.txt");
        std::fs::write(&file_path, "init").unwrap();
        let mut cmd = Command::new("git");
        cmd.args(["add", "init.txt"]).current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());
        let mut cmd = Command::new("git");
        cmd.args(["commit", "-m", "init"]).current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        let req_feat = GitCheckoutRequest {
            branch: "feat".into(),
            create_if_missing: true,
        };
        let res_feat = checkout_branch(&td.path, &req_feat).await.unwrap();
        assert_eq!(res_feat.branch, "feat");

        let req_main = GitCheckoutRequest {
            branch: "main".into(),
            create_if_missing: true,
        };
        let res_main = checkout_branch(&td.path, &req_main).await.unwrap();
        assert_eq!(res_main.branch, "main");
        assert_eq!(res_main.previous_branch, Some("feat".to_string()));
        assert!(res_main.switched);
    }

    #[tokio::test]
    async fn checkout_failure_surfaces_stderr() {
        let td = create_test_repo().await;
        let file_path = td.path.join("init.txt");
        std::fs::write(&file_path, "init").unwrap();
        let mut cmd = Command::new("git");
        cmd.args(["add", "init.txt"]).current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());
        let mut cmd = Command::new("git");
        cmd.args(["commit", "-m", "init"]).current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        let req = GitCheckoutRequest {
            branch: "nonexistent_branch".into(),
            create_if_missing: false,
        };
        let res = checkout_branch(&td.path, &req).await;
        match res {
            Err(GitError::Execution(stderr)) => {
                assert!(
                    stderr.starts_with("git switch failed:"),
                    "expected 'git switch failed:', got: {stderr}"
                );
                assert!(
                    stderr.contains("nonexistent_branch") || stderr.contains("fatal"),
                    "expected git stderr content in: {stderr}"
                );
            }
            other => panic!("expected Err(GitError::Execution), got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sync_rejects_option_like_remote() {
        let td = create_test_repo().await;
        let marker = td.path.join("marker_pwned");
        let evil_remote = format!("--upload-pack=touch {};git-upload-pack", marker.display());

        let req = GitSyncRequest {
            action: "pull".into(),
            remote: Some(evil_remote),
            branch: Some("main".into()),
        };
        let res = sync_repo(&td.path, &req).await;
        assert!(matches!(res, Err(GitError::Validation(_))));
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn unknown_remote_rejected() {
        let td = create_test_repo().await;
        let req = GitSyncRequest {
            action: "pull".into(),
            remote: Some("unknown_remote".into()),
            branch: Some("main".into()),
        };
        let res = sync_repo(&td.path, &req).await;
        match res {
            Err(GitError::Validation(msg)) => {
                assert!(msg.contains("unknown remote 'unknown_remote'"));
            }
            other => panic!("expected Err(GitError::Validation), got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn dash_branch_rejected() {
        let td = create_test_repo().await;
        let req = GitCheckoutRequest {
            branch: "-b".into(),
            create_if_missing: false,
        };
        let res = checkout_branch(&td.path, &req).await;
        assert!(matches!(res, Err(GitError::Validation(_))));

        let res_val = validate_branch("-main").await;
        assert!(matches!(res_val, Err(GitError::Validation(_))));
    }

    #[tokio::test]
    async fn commit_with_paths_leaves_other_staged_files() {
        let td = create_test_repo().await;
        let f1 = td.path.join("file1.txt");
        let f2 = td.path.join("file2.txt");
        std::fs::write(&f1, "f1 initial").unwrap();
        std::fs::write(&f2, "f2 initial").unwrap();

        let mut cmd = Command::new("git");
        cmd.args(["add", "file1.txt", "file2.txt"])
            .current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());
        let mut cmd = Command::new("git");
        cmd.args(["commit", "-m", "init"]).current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        std::fs::write(&f1, "f1 modified").unwrap();
        std::fs::write(&f2, "f2 modified").unwrap();

        let mut cmd = Command::new("git");
        cmd.args(["add", "file2.txt"]).current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        let req = GitCommitRequest {
            message: "commit f1 only".into(),
            paths: Some(vec!["file1.txt".into()]),
            include_untracked: false,
        };
        commit_changes(&td.path, &req).await.unwrap();

        let status = get_repo_status(&td.path).await.unwrap();
        assert!(!status.is_clean);
        assert!(status.files.contains_key("file2.txt"));
        assert!(!status.files.contains_key("file1.txt"));
    }

    #[tokio::test]
    async fn commit_without_paths_commits_all_tracked() {
        let td = create_test_repo().await;
        let f1 = td.path.join("file1.txt");
        let f2 = td.path.join("file2.txt");
        std::fs::write(&f1, "f1 initial").unwrap();
        std::fs::write(&f2, "f2 initial").unwrap();

        let mut cmd = Command::new("git");
        cmd.args(["add", "file1.txt", "file2.txt"])
            .current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());
        let mut cmd = Command::new("git");
        cmd.args(["commit", "-m", "init"]).current_dir(&td.path);
        assert!(cmd.status().await.unwrap().success());

        std::fs::write(&f1, "f1 modified").unwrap();
        std::fs::write(&f2, "f2 modified").unwrap();

        let req = GitCommitRequest {
            message: "commit all tracked".into(),
            paths: None,
            include_untracked: false,
        };
        commit_changes(&td.path, &req).await.unwrap();

        let status = get_repo_status(&td.path).await.unwrap();
        assert!(status.is_clean);
    }

    #[tokio::test]
    async fn push_reports_count() {
        let remote_td = TestDir::new();
        let mut cmd = Command::new("git");
        cmd.args(["init", "-b", "main", "--bare"])
            .current_dir(&remote_td.path);
        assert!(cmd.status().await.unwrap().success());

        let client_td = create_test_repo().await;
        let remote_url = format!("file://{}", remote_td.path.display());
        let mut cmd = Command::new("git");
        cmd.args(["remote", "add", "origin", &remote_url])
            .current_dir(&client_td.path);
        assert!(cmd.status().await.unwrap().success());

        let f = client_td.path.join("test.txt");
        std::fs::write(&f, "c1").unwrap();
        let mut cmd = Command::new("git");
        cmd.args(["add", "test.txt"]).current_dir(&client_td.path);
        assert!(cmd.status().await.unwrap().success());
        let mut cmd = Command::new("git");
        cmd.args(["commit", "-m", "c1"])
            .current_dir(&client_td.path);
        assert!(cmd.status().await.unwrap().success());

        std::fs::write(&f, "c2").unwrap();
        let mut cmd = Command::new("git");
        cmd.args(["commit", "-a", "-m", "c2"])
            .current_dir(&client_td.path);
        assert!(cmd.status().await.unwrap().success());

        let req = GitSyncRequest {
            action: "push".into(),
            remote: Some("origin".into()),
            branch: Some("main".into()),
        };
        let res = sync_repo(&client_td.path, &req).await.unwrap();
        assert_eq!(res.pushed_commits, 2);
        assert!(
            res.output.contains("main -> main"),
            "expected output to contain 'main -> main', got: {}",
            res.output
        );
    }

    #[tokio::test]
    async fn branches_skip_origin_head_symref() {
        let remote_td = TestDir::new();
        let mut cmd = Command::new("git");
        cmd.args(["init", "-b", "main", "--bare"])
            .current_dir(&remote_td.path);
        assert!(cmd.status().await.unwrap().success());

        let client_td = create_test_repo().await;
        let remote_url = format!("file://{}", remote_td.path.display());
        let mut cmd = Command::new("git");
        cmd.args(["remote", "add", "origin", &remote_url])
            .current_dir(&client_td.path);
        assert!(cmd.status().await.unwrap().success());

        let f = client_td.path.join("test.txt");
        std::fs::write(&f, "init").unwrap();
        let mut cmd = Command::new("git");
        cmd.args(["add", "test.txt"]).current_dir(&client_td.path);
        assert!(cmd.status().await.unwrap().success());
        let mut cmd = Command::new("git");
        cmd.args(["commit", "-m", "init"])
            .current_dir(&client_td.path);
        assert!(cmd.status().await.unwrap().success());

        let mut cmd = Command::new("git");
        cmd.args(["push", "-u", "origin", "main"])
            .current_dir(&client_td.path);
        assert!(cmd.status().await.unwrap().success());

        let mut cmd = Command::new("git");
        cmd.args(["remote", "set-head", "origin", "main"])
            .current_dir(&client_td.path);
        assert!(cmd.status().await.unwrap().success());

        let branches = get_repo_branches(&client_td.path).await.unwrap();

        assert!(
            branches.iter().any(|b| b.name == "main" && !b.is_remote),
            "local main branch should be present"
        );
        assert!(
            branches
                .iter()
                .any(|b| b.name == "origin/main" && b.is_remote),
            "remote origin/main branch should be present"
        );
        assert!(
            !branches.iter().any(|b| b.name.contains("HEAD")),
            "origin/HEAD symref should be skipped"
        );
        assert!(
            !branches.iter().any(|b| b.name == "origin"),
            "phantom origin branch should not exist"
        );
    }
}
