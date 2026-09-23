//! Git domain models, repository status, commit payloads, and porcelain parsing.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Git working tree status for a specific file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitFileStatus {
    Added,
    Modified,
    Deleted,
    Untracked,
    Renamed,
    Conflict,
}

impl GitFileStatus {
    /// Return a single-character status badge for the file explorer tree.
    pub fn badge(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Untracked => "U",
            Self::Renamed => "R",
            Self::Conflict => "!",
        }
    }

    /// Return a CSS class modifier for styling file tree badges.
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Added => "git-badge-added",
            Self::Modified => "git-badge-modified",
            Self::Deleted => "git-badge-deleted",
            Self::Untracked => "git-badge-untracked",
            Self::Renamed => "git-badge-renamed",
            Self::Conflict => "git-badge-conflict",
        }
    }
}

/// Aggregated diff insertion and deletion line statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GitLineStats {
    pub insertions: usize,
    pub deletions: usize,
}

impl GitLineStats {
    pub fn format_diff(&self) -> String {
        format!("+{} -{}", self.insertions, self.deletions)
    }
}

/// Comprehensive Git repository status returned to the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitRepoStatus {
    pub branch: String,
    pub commit_hash: String,
    pub commit_message: Option<String>,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub is_clean: bool,
    pub line_stats: GitLineStats,
    /// Map of workspace-relative paths to file status.
    pub files: HashMap<String, GitFileStatus>,
}

impl Default for GitRepoStatus {
    fn default() -> Self {
        Self {
            branch: "main".into(),
            commit_hash: String::new(),
            commit_message: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            is_clean: true,
            line_stats: GitLineStats::default(),
            files: HashMap::new(),
        }
    }
}

/// Request payload to create a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommitRequest {
    pub message: String,
    /// Specific paths to stage and commit, or None to commit all tracked modified/deleted files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
    /// Auto-stage untracked files before committing.
    #[serde(default)]
    pub include_untracked: bool,
}

/// Result returned from a commit operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCommitResult {
    pub commit_hash: String,
    pub summary: String,
    pub pre_commit_output: Option<String>,
    pub is_signed: bool,
}

/// Information about a Git branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitBranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

/// Request payload to switch or create a branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCheckoutRequest {
    pub branch: String,
    #[serde(default)]
    pub create_if_missing: bool,
}

/// Result returned from a checkout operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitCheckoutResult {
    pub branch: String,
    pub previous_branch: Option<String>,
    pub switched: bool,
}

/// Request payload to synchronize with remote upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSyncRequest {
    /// Action to perform: "pull", "push", or "sync" (pull --rebase then push).
    #[serde(default = "default_sync_action")]
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

fn default_sync_action() -> String {
    "sync".into()
}

/// Result returned from remote sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSyncResult {
    pub remote: String,
    pub branch: String,
    pub pulled_commits: usize,
    pub pushed_commits: usize,
    pub output: String,
}

/// Parse output of `git status --porcelain=v1 -b`.
///
/// Returns:
/// `(files_map, branch_name, upstream_branch, ahead_count, behind_count)`
pub fn parse_porcelain_v1(
    output: &str,
) -> (
    HashMap<String, GitFileStatus>,
    String,
    Option<String>,
    usize,
    usize,
) {
    let mut files = HashMap::new();
    let mut branch = String::from("HEAD");
    let mut upstream = None;
    let mut ahead = 0;
    let mut behind = 0;

    for line in output.lines() {
        if line.starts_with("##") {
            // Branch header: e.g.
            // ## main...origin/main [ahead 1, behind 2]
            // ## feat/test
            // ## HEAD (no branch)
            let header = line.trim_start_matches('#').trim();
            if let Some((b_part, tracker)) = header.split_once(" [") {
                parse_branch_part(b_part, &mut branch, &mut upstream);
                let track_clean = tracker.trim_end_matches(']');
                parse_ahead_behind(track_clean, &mut ahead, &mut behind);
            } else {
                parse_branch_part(header, &mut branch, &mut upstream);
            }
        } else if line.len() >= 3 {
            let index_status = line.as_bytes()[0];
            let work_status = line.as_bytes()[1];
            let raw_path = &line[3..].trim();

            let path = if let Some((_, dest)) = raw_path.split_once(" -> ") {
                dest.trim().to_string()
            } else {
                raw_path.to_string()
            };

            let status = if index_status == b'?' || work_status == b'?' {
                GitFileStatus::Untracked
            } else if index_status == b'U'
                || work_status == b'U'
                || (index_status == b'A' && work_status == b'A')
            {
                GitFileStatus::Conflict
            } else if index_status == b'A' {
                GitFileStatus::Added
            } else if index_status == b'D' || work_status == b'D' {
                GitFileStatus::Deleted
            } else if index_status == b'R' {
                GitFileStatus::Renamed
            } else {
                GitFileStatus::Modified
            };

            files.insert(path, status);
        }
    }

    (files, branch, upstream, ahead, behind)
}

fn parse_branch_part(b_part: &str, branch: &mut String, upstream: &mut Option<String>) {
    if let Some((b, u)) = b_part.split_once("...") {
        *branch = b.trim().to_string();
        *upstream = Some(u.trim().to_string());
    } else {
        *branch = b_part.trim().to_string();
    }
}

fn parse_ahead_behind(tracker: &str, ahead: &mut usize, behind: &mut usize) {
    for part in tracker.split(',') {
        let p = part.trim();
        if let Some(n) = p.strip_prefix("ahead ") {
            *ahead = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = p.strip_prefix("behind ") {
            *behind = n.trim().parse().unwrap_or(0);
        }
    }
}

/// Parse net line changes from unified diff or diffstat.
pub fn parse_diff_stat(output: &str) -> GitLineStats {
    let mut insertions = 0;
    let mut deletions = 0;

    for line in output.lines() {
        // e.g. " 3 files changed, 24 insertions(+), 5 deletions(-)"
        // or short stat from diff
        if line.contains("changed") {
            for part in line.split(',') {
                let p = part.trim();
                if p.contains("insertion") {
                    if let Some(num_str) = p.split_whitespace().next() {
                        insertions = num_str.parse().unwrap_or(0);
                    }
                } else if p.contains("deletion")
                    && let Some(num_str) = p.split_whitespace().next()
                {
                    deletions = num_str.parse().unwrap_or(0);
                }
            }
        }
    }

    GitLineStats {
        insertions,
        deletions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_porcelain_v1_full() {
        let output = "## main...origin/main [ahead 2, behind 1]\n M crates/core/src/lib.rs\n?? new_file.txt\n D old_file.rs\nA  staged.rs\nR  old.txt -> new.txt\nUU conflict.rs\n";
        let (files, branch, upstream, ahead, behind) = parse_porcelain_v1(output);

        assert_eq!(branch, "main");
        assert_eq!(upstream, Some("origin/main".to_string()));
        assert_eq!(ahead, 2);
        assert_eq!(behind, 1);

        assert_eq!(
            files.get("crates/core/src/lib.rs"),
            Some(&GitFileStatus::Modified)
        );
        assert_eq!(files.get("new_file.txt"), Some(&GitFileStatus::Untracked));
        assert_eq!(files.get("old_file.rs"), Some(&GitFileStatus::Deleted));
        assert_eq!(files.get("staged.rs"), Some(&GitFileStatus::Added));
        assert_eq!(files.get("new.txt"), Some(&GitFileStatus::Renamed));
        assert_eq!(files.get("conflict.rs"), Some(&GitFileStatus::Conflict));
    }

    #[test]
    fn test_parse_porcelain_v1_clean() {
        let output = "## feat/awesome\n";
        let (files, branch, upstream, ahead, behind) = parse_porcelain_v1(output);

        assert_eq!(branch, "feat/awesome");
        assert_eq!(upstream, None);
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
        assert!(files.is_empty());
    }

    #[test]
    fn test_parse_diff_stat() {
        let output = " 2 files changed, 42 insertions(+), 12 deletions(-)\n";
        let stats = parse_diff_stat(output);
        assert_eq!(stats.insertions, 42);
        assert_eq!(stats.deletions, 12);
        assert_eq!(stats.format_diff(), "+42 -12");
    }
}
