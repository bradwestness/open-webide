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

/// Decode NUL records once, consuming rename/copy source records together.
fn porcelain_records(output: &str) -> impl Iterator<Item = (&str, Option<&str>)> {
    let mut records = output.split('\0');
    std::iter::from_fn(move || {
        let record = records.next()?;
        let bytes = record.as_bytes();
        let source = if bytes.len() >= 4
            && bytes[2] == b' '
            && bytes[..2]
                .iter()
                .any(|status| matches!(status, b'R' | b'C'))
        {
            records.next()
        } else {
            None
        };
        Some((record, source))
    })
}

/// Parse output of `git status --porcelain=v1 -b -z`.
///
/// Records are NUL-separated and paths are unquoted. A rename/copy record is
/// followed by a second record holding the source path. Returns:
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

    for (rec, _) in porcelain_records(output) {
        if rec.starts_with("## ") {
            // Branch header: e.g.
            // ## main...origin/main [ahead 1, behind 2]
            // ## feat/test
            // ## No commits yet on main
            // ## HEAD (no branch)
            let header = rec.trim_start_matches('#').trim();
            if let Some(name) = header
                .strip_prefix("No commits yet on ")
                .or_else(|| header.strip_prefix("Initial commit on "))
            {
                branch = name.trim().to_string();
            } else if header == "HEAD (no branch)" {
                branch = "HEAD".to_string();
            } else if let Some((b_part, tracker)) = header.split_once(" [") {
                parse_branch_part(b_part, &mut branch, &mut upstream);
                let track_clean = tracker.trim_end_matches(']');
                parse_ahead_behind(track_clean, &mut ahead, &mut behind);
            } else {
                parse_branch_part(header, &mut branch, &mut upstream);
            }
            continue;
        }

        let bytes = rec.as_bytes();
        if rec.len() < 4 || bytes[2] != b' ' {
            continue;
        }
        let index_status = bytes[0];
        let work_status = bytes[1];
        let path = rec.get(3..).unwrap_or("");

        let status = if matches!(
            (index_status, work_status),
            (b'D' | b'U', b'D') | (b'A' | b'D' | b'U', b'U') | (b'U' | b'A', b'A')
        ) {
            GitFileStatus::Conflict
        } else if index_status == b'?' || work_status == b'?' {
            GitFileStatus::Untracked
        } else if index_status == b'A' {
            GitFileStatus::Added
        } else if index_status == b'D' || work_status == b'D' {
            GitFileStatus::Deleted
        } else if index_status == b'R' {
            GitFileStatus::Renamed
        } else {
            GitFileStatus::Modified
        };

        files.insert(path.to_string(), status);
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

/// Diff payload shared by bridge HTTP and in-process Git adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitDiff {
    pub diff: String,
}

/// A file's contents at HEAD. Missing encoding preserves legacy UTF-8 responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitFileContent {
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

impl GitFileContent {
    /// Text previews share binary and unknown-encoding behavior across transports.
    pub fn into_text(self) -> Result<String, String> {
        match self.encoding.as_deref().unwrap_or("utf8") {
            "utf8" => Ok(self.content),
            "base64" => Err("binary file".into()),
            encoding => Err(format!("unknown blob encoding: {encoding}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_text_preview_contract() {
        for encoding in [None, Some("utf8")] {
            let value = GitFileContent {
                content: "世界\n".into(),
                encoding: encoding.map(str::to_string),
            };
            assert_eq!(value.into_text().unwrap(), "世界\n");
        }
        let binary = GitFileContent {
            content: "/w==".into(),
            encoding: Some("base64".into()),
        };
        assert_eq!(binary.into_text().unwrap_err(), "binary file");
        let unknown = GitFileContent {
            content: "text".into(),
            encoding: Some("unknown".into()),
        };
        assert!(unknown.into_text().is_err());
    }

    #[test]
    fn test_parse_porcelain_v1_full() {
        // -z output: NUL-separated records, unquoted paths; a rename record
        // is followed by a record holding the source path.
        let output = "## main...origin/main [ahead 2, behind 1]\0 M crates/core/src/lib.rs\0?? new file with space.txt\0A  файл.txt\0 D old_file.rs\0A  staged.rs\0R  new.txt\0old.txt\0DD both_deleted.rs\0Mbad\0";
        let (files, branch, upstream, ahead, behind) = parse_porcelain_v1(output);

        assert_eq!(branch, "main");
        assert_eq!(upstream, Some("origin/main".to_string()));
        assert_eq!(ahead, 2);
        assert_eq!(behind, 1);

        assert_eq!(
            files.get("crates/core/src/lib.rs"),
            Some(&GitFileStatus::Modified)
        );
        assert_eq!(
            files.get("new file with space.txt"),
            Some(&GitFileStatus::Untracked)
        );
        assert_eq!(files.get("файл.txt"), Some(&GitFileStatus::Added));
        assert_eq!(files.get("old_file.rs"), Some(&GitFileStatus::Deleted));
        assert_eq!(files.get("staged.rs"), Some(&GitFileStatus::Added));
        assert_eq!(files.get("new.txt"), Some(&GitFileStatus::Renamed));
        assert!(
            !files.contains_key("old.txt"),
            "rename source must be consumed"
        );
        assert_eq!(files.get("both_deleted.rs"), Some(&GitFileStatus::Conflict));
        assert!(
            !files.contains_key("Mbad"),
            "malformed record must be skipped"
        );
    }

    #[test]
    fn test_parse_porcelain_v1_clean() {
        let output = "## feat/awesome\0";
        let (files, branch, upstream, ahead, behind) = parse_porcelain_v1(output);

        assert_eq!(branch, "feat/awesome");
        assert_eq!(upstream, None);
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
        assert!(files.is_empty());

        // Unborn-branch headers carry the branch name after a prefix.
        let (files, branch, ..) = parse_porcelain_v1("## No commits yet on main\0");
        assert_eq!(branch, "main");
        assert!(files.is_empty());

        let (files, branch, ..) = parse_porcelain_v1("## Initial commit on main\0");
        assert_eq!(branch, "main");
        assert!(files.is_empty());

        // Detached HEAD reports the literal branch "HEAD".
        let (files, branch, ..) = parse_porcelain_v1("## HEAD (no branch)\0");
        assert_eq!(branch, "HEAD");
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

/// File-tree Git mutations; each request names one literal workspace path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitPathAction {
    Stage,
    Unstage,
    Revert,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitPathRequest {
    pub path: String,
    pub action: GitPathAction,
}

/// Index and working-tree changes remain separate so menus can offer both
/// Stage and Unstage when a staged file has been edited again.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GitPathChanges {
    #[serde(default)]
    pub has_head: bool,
    pub staged: std::collections::BTreeSet<String>,
    pub unstaged: std::collections::BTreeSet<String>,
    pub untracked: std::collections::BTreeSet<String>,
    pub renamed_from: HashMap<String, String>,
}

impl GitPathChanges {
    pub fn has_staged(&self, path: &str) -> bool {
        self.staged
            .iter()
            .any(|entry| crate::workspace_entries::contains_path(path, entry))
    }
    pub fn has_unstaged(&self, path: &str) -> bool {
        self.unstaged
            .iter()
            .chain(&self.untracked)
            .any(|entry| crate::workspace_entries::contains_path(path, entry))
    }
    pub fn has_tracked_changes(&self, path: &str) -> bool {
        self.staged
            .iter()
            .chain(&self.unstaged)
            .any(|entry| crate::workspace_entries::contains_path(path, entry))
    }
    /// Include a renamed entry's original name to unstage/revert the whole
    /// rename rather than leaving its staged deletion behind.
    pub fn action_paths(&self, path: &str) -> Result<Vec<String>, String> {
        let path = crate::workspace_entries::entry_path(path)?;
        let mut paths = vec![path.clone()];
        for (destination, source) in &self.renamed_from {
            if crate::workspace_entries::contains_path(&path, destination) {
                paths.push(crate::workspace_entries::entry_path(source)?);
            }
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// Parse the same NUL-delimited porcelain records as repository status,
/// retaining the two status columns and rename source names.
pub fn parse_path_changes(output: &str) -> GitPathChanges {
    let mut changes = GitPathChanges::default();
    for (record, source) in porcelain_records(output) {
        if record.starts_with("## ") {
            continue;
        }
        let bytes = record.as_bytes();
        if bytes.len() < 4 || bytes[2] != b' ' {
            continue;
        }
        let path = &record[3..];
        if bytes[0] == b'?' && bytes[1] == b'?' {
            changes.untracked.insert(path.into());
        } else {
            if bytes[0] != b' ' {
                changes.staged.insert(path.into());
            }
            if bytes[1] != b' ' {
                changes.unstaged.insert(path.into());
            }
        }
        // Copies do not delete their source and must not revert it.
        if bytes[..2].contains(&b'R')
            && let Some(source) = source
        {
            changes.renamed_from.insert(path.into(), source.into());
        }
    }
    changes
}

/// Domain validation and command choice shared by every Git transport.
/// The execution adapter must set GIT_LITERAL_PATHSPECS=1.
pub fn path_action_args(
    request: &GitPathRequest,
    changes: &GitPathChanges,
    has_head: bool,
) -> Result<Vec<String>, String> {
    let path = crate::workspace_entries::entry_path(&request.path)?;
    let paths = changes.action_paths(&path)?;
    if request.action == GitPathAction::Revert
        && changes.untracked.iter().any(|untracked| {
            changes
                .staged
                .iter()
                .chain(&changes.unstaged)
                .chain(changes.renamed_from.values())
                .filter(|tracked| {
                    paths
                        .iter()
                        .any(|path| crate::workspace_entries::contains_path(path, tracked))
                })
                .any(|tracked| {
                    crate::workspace_entries::contains_path(tracked, untracked)
                        || crate::workspace_entries::contains_path(untracked, tracked)
                })
        })
    {
        return Err("Move untracked files out of the paths being restored before reverting".into());
    }
    let args = match request.action {
        GitPathAction::Stage if changes.has_unstaged(&path) => vec!["add", "-A", "--"],
        GitPathAction::Unstage if changes.has_staged(&path) => {
            if has_head {
                vec!["reset", "HEAD", "--"]
            } else {
                vec!["rm", "-r", "--cached", "--ignore-unmatch", "--"]
            }
        }
        GitPathAction::Revert if has_head && changes.has_tracked_changes(&path) => {
            vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"]
        }
        _ => return Err("This Git action is not available for the selected path".into()),
    };
    let mut args = args.into_iter().map(str::to_owned).collect::<Vec<_>>();
    args.extend(paths);
    Ok(args)
}

#[cfg(test)]
mod path_tests {
    use super::*;
    #[test]
    fn distinguishes_index_and_worktree_and_preserves_literal_names() {
        let changes = parse_path_changes(
            "## main\0MM src/a b\0?? src/[x]*\0R  new\0old\0C  copied\0source\0",
        );
        assert!(changes.has_staged("src"));
        assert!(changes.has_unstaged("src"));
        assert!(!changes.has_unstaged("sr"));
        assert_eq!(changes.action_paths("new").unwrap(), ["new", "old"]);
        assert_eq!(changes.action_paths("copied").unwrap(), ["copied"]);
        let args = path_action_args(
            &GitPathRequest {
                path: "src/[x]*".into(),
                action: GitPathAction::Stage,
            },
            &changes,
            true,
        )
        .unwrap();
        assert_eq!(args, ["add", "-A", "--", "src/[x]*"]);
    }
    #[test]
    fn unborn_unstage_keeps_worktree_and_revert_never_cleans_untracked() {
        let changes = parse_path_changes("A  new\0?? scratch\0");
        let request = GitPathRequest {
            path: "new".into(),
            action: GitPathAction::Unstage,
        };
        assert_eq!(
            path_action_args(&request, &changes, false).unwrap(),
            ["rm", "-r", "--cached", "--ignore-unmatch", "--", "new"]
        );
        for (path, has_head) in [("new", false), ("scratch", true)] {
            assert!(
                path_action_args(
                    &GitPathRequest {
                        path: path.into(),
                        action: GitPathAction::Revert
                    },
                    &changes,
                    has_head
                )
                .is_err()
            );
        }
        assert!(changes.action_paths("../outside").is_err());
    }
}
