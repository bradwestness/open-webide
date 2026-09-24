//! Universal VFS tool executor for the agent loop.
//!
//! Executes workspace tools (`read_file`, `write_file`, `list_dir`, `search`, `grep_search`)
//! against any implementation of the [`Vfs`] trait.

use std::future::Future;

use openwebide_core::{
    CommandOutcome, FileDiff, FileEntry, GitCheckoutRequest, GitCheckoutResult, GitCommitRequest,
    GitCommitResult, GitRepoStatus, ToolCall, ToolDefinition, Vfs, VfsError, WebSearchResult,
    normalize_vfs_path, vfs::SearchOptions,
};
use serde_json::{Value, json};

use crate::{ToolExecutor, ToolOutcome};

/// The standard workspace tools offered to the agent model.
pub fn vfs_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read_file".into(),
            description: "Read the contents of a file in the workspace.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative file path" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "write_file".into(),
            description: "Create or overwrite a file in the workspace with the given full contents.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative file path" },
                    "content": { "type": "string", "description": "Full new contents of the file" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "list_dir".into(),
            description: "List the entries of a directory in the workspace.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Workspace-relative directory path (empty for root)" }
                }
            }),
        },
        ToolDefinition {
            name: "search".into(),
            description: "Search file names in the workspace for a substring.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Substring to match against file paths" },
                    "path": { "type": "string", "description": "Workspace-relative directory to search in (empty for root)" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "grep_search".into(),
            description: "Search workspace file contents for lines matching a substring.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search string to match across file lines" },
                    "path": { "type": "string", "description": "Workspace-relative directory to restrict search (empty for root)" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "search_web".into(),
            description: "Search the web for up-to-date documentation, API references, or error solutions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "integer", "description": "Number of results to return (default: 5, max: 10)" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "fetch_web_page".into(),
            description: "Fetch a web page URL and convert its content to clean Markdown.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Full HTTP or HTTPS URL to read" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "run_command".into(),
            description: "Execute a shell command in the project directory. Use this to run builds, tests, linters, or inspect git status.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute (e.g. 'cargo test', 'git diff')"
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "description": "Maximum execution time in seconds before terminating (default: 30, max: 300)"
                    }
                },
                "required": ["command"]
            }),
        },
        ToolDefinition {
            name: "git_status".into(),
            description: "Inspect uncommitted modifications, untracked files, and current branch status.".into(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "git_diff".into(),
            description: "View the unified diff of uncommitted changes in the repository or for a specific file.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Optional file path to inspect." }
                }
            }),
        },
        ToolDefinition {
            name: "git_commit".into(),
            description: "Create a Git commit on the host with a descriptive conventional commit message. Omit `paths` to commit all tracked modifications.".into(),
            parameters: json!({
                "type": "object",
                "required": ["message"],
                "properties": {
                    "message": { "type": "string", "description": "Conventional commit message (e.g. 'feat(core): add diff parser')." },
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Optional subset of files to commit." }
                }
            }),
        },
        ToolDefinition {
            name: "git_branch".into(),
            description: "Create and checkout a new git feature branch before starting a task.".into(),
            parameters: json!({
                "type": "object",
                "required": ["branch_name"],
                "properties": {
                    "branch_name": { "type": "string", "description": "Name of the new branch (e.g. 'feat/argon2-auth')." }
                }
            }),
        },
    ]
}

/// Web search and documentation fetching capability for the agent.
pub trait WebClient: Send + Sync {
    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<WebSearchResult>, String>> + Send;

    fn fetch_page(&self, url: &str) -> impl Future<Output = Result<String, String>> + Send;
}

/// A no-op web client for environments without external web access.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopWebClient;

impl WebClient for NoopWebClient {
    async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<WebSearchResult>, String> {
        Err("Web access is not configured in this environment.".into())
    }

    async fn fetch_page(&self, _url: &str) -> Result<String, String> {
        Err("Web access is not configured in this environment.".into())
    }
}

/// Process execution bridge client capability for the agent.
pub trait BridgeClient: Send + Sync {
    fn execute_command(
        &self,
        command: &str,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<CommandOutcome, String>> + Send;

    fn git_status(&self) -> impl Future<Output = Result<GitRepoStatus, String>> + Send {
        async { Err("Git status is not available (bridge daemon not connected).".into()) }
    }

    fn git_diff(&self, path: Option<&str>) -> impl Future<Output = Result<String, String>> + Send {
        let _ = path;
        async { Err("Git diff is not available (bridge daemon not connected).".into()) }
    }

    fn git_commit(
        &self,
        req: &GitCommitRequest,
    ) -> impl Future<Output = Result<GitCommitResult, String>> + Send {
        let _ = req;
        async { Err("Git commit is not available (bridge daemon not connected).".into()) }
    }

    fn git_checkout(
        &self,
        req: &GitCheckoutRequest,
    ) -> impl Future<Output = Result<GitCheckoutResult, String>> + Send {
        let _ = req;
        async { Err("Git checkout is not available (bridge daemon not connected).".into()) }
    }
}

/// A no-op bridge client for environments without an active terminal bridge daemon.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopBridgeClient;

impl BridgeClient for NoopBridgeClient {
    async fn execute_command(
        &self,
        _command: &str,
        _timeout_seconds: u64,
    ) -> Result<CommandOutcome, String> {
        Err("Process execution is not available (bridge daemon not connected). Start 'openwebide-bridge' to enable shell commands.".into())
    }
}

/// A tool executor backed by a [`Vfs`], optional [`WebClient`], and optional [`BridgeClient`].
pub struct VfsToolExecutor<V: Vfs, W: WebClient = NoopWebClient, B: BridgeClient = NoopBridgeClient>
{
    vfs: V,
    web: W,
    bridge: B,
}

impl<V: Vfs> VfsToolExecutor<V, NoopWebClient, NoopBridgeClient> {
    pub fn new(vfs: V) -> Self {
        Self {
            vfs,
            web: NoopWebClient,
            bridge: NoopBridgeClient,
        }
    }
}

impl<V: Vfs, W: WebClient> VfsToolExecutor<V, W, NoopBridgeClient> {
    pub fn with_web(vfs: V, web: W) -> Self {
        Self {
            vfs,
            web,
            bridge: NoopBridgeClient,
        }
    }
}

impl<V: Vfs, W: WebClient, B: BridgeClient> VfsToolExecutor<V, W, B> {
    pub fn with_web_and_bridge(vfs: V, web: W, bridge: B) -> Self {
        Self { vfs, web, bridge }
    }

    pub fn vfs(&self) -> &V {
        &self.vfs
    }

    pub fn web(&self) -> &W {
        &self.web
    }

    pub fn bridge(&self) -> &B {
        &self.bridge
    }

    async fn read_file(&self, args: &Value) -> ToolOutcome {
        let raw_path = arg_path(args);
        let path = match normalize_vfs_path(&raw_path) {
            Ok(p) => p,
            Err(e) => return fail("read_file", &raw_path, &e.to_string()),
        };

        match self.vfs.read(&path).await {
            Ok(content) => {
                let lines = content.lines().count();
                ToolOutcome {
                    ok: true,
                    content,
                    summary: format!("read {path} ({lines} lines)"),
                    diff: None,
                }
            }
            Err(e) => fail("read_file", &path, &e.to_string()),
        }
    }

    async fn write_file(&self, args: &Value, step_id: &str) -> ToolOutcome {
        let raw_path = arg_path(args);
        let path = match normalize_vfs_path(&raw_path) {
            Ok(p) => p,
            Err(e) => return fail("write_file", &raw_path, &e.to_string()),
        };
        if path.split('/').any(|seg| seg.eq_ignore_ascii_case(".git")) {
            return fail("write_file", &path, "refusing to write inside .git");
        }
        let canonical_path = match self.vfs.canonicalize(&path).await {
            Ok(p) => p,
            Err(e) => return fail("write_file", &path, &e.to_string()),
        };
        if canonical_path
            .split('/')
            .any(|seg| seg.eq_ignore_ascii_case(".git"))
        {
            return fail("write_file", &path, "refusing to write inside .git");
        }
        let new_content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return fail("write_file", &path, "missing 'content' argument"),
        };

        let mut old = None;
        let mut old_unavailable = false;
        let mut backup_path = None;

        match self.vfs.read(&path).await {
            Ok(c) => old = Some(c),
            Err(VfsError::NotFound(_)) => old = None,
            Err(e) => {
                let file_name = path.rsplit_once('/').map(|(_, f)| f).unwrap_or(&path);
                let backup = format!(
                    "{}/{step_id}/{file_name}",
                    openwebide_core::vfs::AGENT_BACKUP_DIR
                );
                let gitignore = format!("{}/.gitignore", openwebide_core::vfs::AGENT_BACKUP_DIR);

                // Write .gitignore if missing
                if let Err(VfsError::NotFound(_)) = self.vfs.read(&gitignore).await {
                    let _ = self.vfs.write(&gitignore, "*\n").await;
                }

                if let Err(c) = self.vfs.copy(&path, &backup).await {
                    return fail(
                        "write_file",
                        &path,
                        &format!(
                            "refusing to overwrite: the existing file could not be read ({e}) and could not be backed up ({c})"
                        ),
                    );
                }
                old_unavailable = true;
                backup_path = Some(backup);
            }
        }

        match self.vfs.write(&path, new_content).await {
            Ok(()) => {
                let diff = FileDiff {
                    path: path.clone(),
                    old,
                    new: new_content.to_string(),
                    old_unavailable,
                    backup_path,
                };
                let content = if old_unavailable {
                    format!(
                        "wrote {path}; the previous version was not readable as text and was backed up"
                    )
                } else {
                    format!("wrote {path}")
                };
                let summary = if old_unavailable {
                    format!("write {path} (diff unavailable: previous file backed up)")
                } else {
                    format!("wrote {path}")
                };
                ToolOutcome {
                    ok: true,
                    content,
                    summary,
                    diff: Some(diff),
                }
            }
            Err(e) => fail("write_file", &path, &e.to_string()),
        }
    }

    async fn list_dir(&self, args: &Value) -> ToolOutcome {
        let raw_dir = arg_path_or_root(args);
        let dir = match normalize_vfs_path(&raw_dir) {
            Ok(p) => p,
            Err(e) => return fail("list_dir", &raw_dir, &e.to_string()),
        };

        match self.vfs.list(&dir).await {
            Ok(entries) => {
                let count = entries.len();
                if entries.is_empty() {
                    return ToolOutcome {
                        ok: true,
                        content: "(empty directory)".to_string(),
                        summary: format!("list {dir} (0 entries)"),
                        diff: None,
                    };
                }

                let mut out = String::new();
                for entry in entries {
                    if entry.is_dir {
                        out.push_str(&format!("[DIR]  {}/\n", entry.name));
                    } else {
                        out.push_str(&format!("[FILE] {} ({} B)\n", entry.name, entry.size));
                    }
                }
                ToolOutcome {
                    ok: true,
                    content: out,
                    summary: format!("list {dir} ({count} entries)"),
                    diff: None,
                }
            }
            Err(e) => fail("list_dir", &dir, &e.to_string()),
        }
    }

    async fn search(&self, args: &Value) -> ToolOutcome {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        if query.is_empty() {
            return fail("search", "", "missing 'query' argument");
        }
        let raw_path = arg_path_or_root(args);
        let dir = match normalize_vfs_path(&raw_path) {
            Ok(p) => p,
            Err(e) => return fail("search", &raw_path, &e.to_string()),
        };

        match recursive_list(&self.vfs, &dir).await {
            Ok(entries) => {
                let query_lower = query.to_lowercase();
                let matches: Vec<_> = entries
                    .into_iter()
                    .filter(|e| e.name.to_lowercase().contains(&query_lower))
                    .collect();

                let count = matches.len();
                let content = if matches.is_empty() {
                    format!("no files matching '{query}'")
                } else {
                    matches
                        .into_iter()
                        .map(|e| e.path)
                        .collect::<Vec<_>>()
                        .join("\n")
                };

                ToolOutcome {
                    ok: true,
                    content,
                    summary: format!("search '{query}' ({count} matches)"),
                    diff: None,
                }
            }
            Err(e) => fail("search", &dir, &e.to_string()),
        }
    }

    async fn grep_search(&self, args: &Value) -> ToolOutcome {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        if query.is_empty() {
            return fail("grep_search", "", "missing 'query' argument");
        }
        let raw_path = arg_path_or_root(args);
        let dir = match normalize_vfs_path(&raw_path) {
            Ok(p) => p,
            Err(e) => return fail("grep_search", &raw_path, &e.to_string()),
        };

        match self
            .vfs
            .search_content(query, &dir, SearchOptions::default())
            .await
        {
            Ok(hits) => {
                let count = hits.len();
                let content = if hits.is_empty() {
                    format!("no matches found for '{query}'")
                } else {
                    hits.into_iter()
                        .map(|h| format!("{}:{}: {}", h.path, h.line, h.text))
                        .collect::<Vec<_>>()
                        .join("\n")
                };

                ToolOutcome {
                    ok: true,
                    content,
                    summary: format!("grep '{query}' ({count} matches)"),
                    diff: None,
                }
            }
            Err(e) => fail("grep_search", &dir, &e.to_string()),
        }
    }

    async fn search_web(&self, args: &Value) -> ToolOutcome {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return fail("search_web", "", "missing 'query' argument");
        }
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(5)
            .clamp(1, 10);

        match self.web.search(query, limit).await {
            Ok(results) => {
                let count = results.len();
                if results.is_empty() {
                    return ToolOutcome {
                        ok: true,
                        content: format!("no web results found for '{query}'"),
                        summary: format!("search web '{query}' (0 results)"),
                        diff: None,
                    };
                }
                let mut out = String::new();
                for r in results {
                    out.push_str(&format!(
                        "Title: {}\nURL: {}\nSnippet: {}\n---\n",
                        r.title, r.url, r.snippet
                    ));
                }
                ToolOutcome {
                    ok: true,
                    content: out.trim_end_matches("\n---\n").to_string(),
                    summary: format!("search web '{query}' ({count} results)"),
                    diff: None,
                }
            }
            Err(e) => fail("search_web", query, &e),
        }
    }

    async fn fetch_web_page(&self, args: &Value) -> ToolOutcome {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if url.is_empty() {
            return fail("fetch_web_page", "", "missing 'url' argument");
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return fail(
                "fetch_web_page",
                url,
                "URL must start with http:// or https://",
            );
        }

        match self.web.fetch_page(url).await {
            Ok(markdown) => {
                let bytes = markdown.len();
                ToolOutcome {
                    ok: true,
                    content: markdown,
                    summary: format!("fetch {url} ({bytes} bytes)"),
                    diff: None,
                }
            }
            Err(e) => fail("fetch_web_page", url, &e),
        }
    }

    async fn run_command(&self, args: &Value) -> ToolOutcome {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if command.is_empty() {
            return fail("run_command", "", "missing 'command' argument");
        }
        let timeout_seconds = args
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(30)
            .clamp(1, 300);

        match self.bridge.execute_command(command, timeout_seconds).await {
            Ok(outcome) => {
                let summary = match outcome.exit_code {
                    Some(0) => format!("ran '{command}' (exit 0)"),
                    Some(code) => format!("ran '{command}' (exit {code})"),
                    None => format!("ran '{command}' (terminated)"),
                };
                ToolOutcome {
                    ok: outcome.exit_code == Some(0),
                    content: outcome.to_model_text(),
                    summary,
                    diff: None,
                }
            }
            Err(e) => fail("run_command", command, &e),
        }
    }

    async fn git_status(&self) -> ToolOutcome {
        match self.bridge.git_status().await {
            Ok(status) => {
                let commit_short = if status.commit_hash.len() > 7 {
                    &status.commit_hash[..7]
                } else {
                    &status.commit_hash
                };
                let mut out = format!(
                    "Branch: {}\nCommit: {}\nUpstream: {} (ahead {}, behind {})\nClean: {}\nChanges: +{} -{}\n",
                    status.branch,
                    commit_short,
                    status.upstream.as_deref().unwrap_or("(none)"),
                    status.ahead,
                    status.behind,
                    status.is_clean,
                    status.line_stats.insertions,
                    status.line_stats.deletions,
                );
                if !status.files.is_empty() {
                    out.push_str("\nChanged files:\n");
                    for (file, file_status) in &status.files {
                        out.push_str(&format!("  {} {}\n", file_status.badge(), file));
                    }
                }
                let summary = format!(
                    "git status: branch {}, {} changed files",
                    status.branch,
                    status.files.len()
                );
                ToolOutcome {
                    ok: true,
                    content: out,
                    summary,
                    diff: None,
                }
            }
            Err(e) => fail("git_status", "", &e),
        }
    }

    async fn git_diff(&self, args: &Value) -> ToolOutcome {
        let path = args.get("path").and_then(|v| v.as_str());
        match self.bridge.git_diff(path).await {
            Ok(diff) => {
                let content = if diff.trim().is_empty() {
                    "No changes detected (clean working tree).".to_string()
                } else {
                    diff
                };
                let summary = match path {
                    Some(p) => format!("git diff '{p}'"),
                    None => "git diff (repository)".to_string(),
                };
                ToolOutcome {
                    ok: true,
                    content,
                    summary,
                    diff: None,
                }
            }
            Err(e) => fail("git_diff", path.unwrap_or(""), &e),
        }
    }

    async fn git_commit(&self, args: &Value) -> ToolOutcome {
        let message = match args.get("message").and_then(|v| v.as_str()) {
            Some(m) if !m.trim().is_empty() => m.trim().to_string(),
            _ => return fail("git_commit", "", "missing or empty 'message' argument"),
        };
        let paths = args
            .get("paths")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .filter(|p| !p.is_empty());

        let req = GitCommitRequest {
            message: message.clone(),
            paths,
            include_untracked: false,
        };

        match self.bridge.git_commit(&req).await {
            Ok(result) => {
                let commit_short = if result.commit_hash.len() > 7 {
                    &result.commit_hash[..7]
                } else {
                    &result.commit_hash
                };
                let summary = format!("committed {commit_short}: '{message}'");
                let mut content = format!(
                    "Commit: {}\nSummary: {}\nSigned: {}\n",
                    result.commit_hash, result.summary, result.is_signed
                );
                if let Some(hooks) = result.pre_commit_output {
                    content.push_str(&format!("\nHook output:\n{hooks}"));
                }
                ToolOutcome {
                    ok: true,
                    content,
                    summary,
                    diff: None,
                }
            }
            Err(e) => fail("git_commit", &message, &e),
        }
    }

    async fn git_branch(&self, args: &Value) -> ToolOutcome {
        let branch_name = match args.get("branch_name").and_then(|v| v.as_str()) {
            Some(b) if !b.trim().is_empty() => b.trim().to_string(),
            _ => return fail("git_branch", "", "missing or empty 'branch_name' argument"),
        };

        let req = GitCheckoutRequest {
            branch: branch_name.clone(),
            create_if_missing: true,
        };

        match self.bridge.git_checkout(&req).await {
            Ok(result) => {
                let summary = format!("switched to branch '{}'", result.branch);
                let prev = result.previous_branch.as_deref().unwrap_or("none");
                ToolOutcome {
                    ok: result.switched,
                    content: format!(
                        "Branch: {}\nPrevious branch: {}\nSwitched: {}",
                        result.branch, prev, result.switched
                    ),
                    summary,
                    diff: None,
                }
            }
            Err(e) => fail("git_branch", &branch_name, &e),
        }
    }
}

impl<V: Vfs, W: WebClient, B: BridgeClient> ToolExecutor for VfsToolExecutor<V, W, B> {
    fn describe(&self, call: &ToolCall) -> String {
        let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
        match call.name.as_str() {
            "read_file" => format!("read {}", arg_path(&args)),
            "write_file" => {
                let path = arg_path(&args);
                let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let n = content.lines().count();
                let bytes = content.len();
                format!("write {path} ({n} lines, {bytes} B)")
            }
            "list_dir" => format!("list {}", arg_path_or_root(&args)),
            "search" => format!(
                "search '{}'",
                args.get("query").and_then(|v| v.as_str()).unwrap_or("")
            ),
            "grep_search" => format!(
                "grep '{}'",
                args.get("query").and_then(|v| v.as_str()).unwrap_or("")
            ),
            "search_web" => format!(
                "search web for '{}'",
                args.get("query").and_then(|v| v.as_str()).unwrap_or("")
            ),
            "fetch_web_page" => format!(
                "fetch web page {}",
                args.get("url").and_then(|v| v.as_str()).unwrap_or("")
            ),
            "run_command" => format!(
                "run '{}'",
                args.get("command").and_then(|v| v.as_str()).unwrap_or("")
            ),
            "git_status" => "inspect git status".to_string(),
            "git_diff" => match args.get("path").and_then(|v| v.as_str()) {
                Some(p) => format!("inspect git diff for '{p}'"),
                None => "inspect repository git diff".to_string(),
            },
            "git_commit" => {
                let msg = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
                let paths: Option<Vec<&str>> = args
                    .get("paths")
                    .and_then(|p| p.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                    .filter(|p| !p.is_empty());
                match paths {
                    Some(ref p) => format!("commit {}: '{msg}'", p.join(", ")),
                    None => format!("commit ALL tracked changes: '{msg}'"),
                }
            }
            "git_branch" => format!(
                "switch to branch '{}'",
                args.get("branch_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            ),
            other => other.to_string(),
        }
    }

    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
        match call.name.as_str() {
            "read_file" => self.read_file(&args).await,
            "write_file" => self.write_file(&args, &call.id).await,
            "list_dir" => self.list_dir(&args).await,
            "search" => self.search(&args).await,
            "grep_search" => self.grep_search(&args).await,
            "search_web" => self.search_web(&args).await,
            "fetch_web_page" => self.fetch_web_page(&args).await,
            "run_command" => self.run_command(&args).await,
            "git_status" => self.git_status().await,
            "git_diff" => self.git_diff(&args).await,
            "git_commit" => self.git_commit(&args).await,
            "git_branch" => self.git_branch(&args).await,
            other => ToolOutcome {
                ok: false,
                content: format!("unknown tool: {other}"),
                summary: format!("unknown tool: {other}"),
                diff: None,
            },
        }
    }
}

fn arg_path(args: &Value) -> String {
    args.get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn arg_path_or_root(args: &Value) -> String {
    args.get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn fail(tool: &str, target: &str, error: &str) -> ToolOutcome {
    let summary = if target.is_empty() {
        format!("{tool} failed: {error}")
    } else {
        format!("{tool} {target} failed: {error}")
    };
    ToolOutcome {
        ok: false,
        content: format!("error: {error}"),
        summary,
        diff: None,
    }
}

async fn recursive_list<V: Vfs>(vfs: &V, dir: &str) -> Result<Vec<FileEntry>, VfsError> {
    let mut all = Vec::new();
    let mut stack = vec![dir.to_string()];
    while let Some(current) = stack.pop() {
        let entries = vfs.list(&current).await?;
        for entry in entries {
            if entry.is_dir {
                stack.push(entry.path.clone());
            }
            all.push(entry);
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::MemoryVfs;

    #[test]
    fn test_vfs_tool_executor_read_and_write() {
        futures::executor::block_on(async {
            let vfs = MemoryVfs::new();
            let executor = VfsToolExecutor::new(vfs);

            // write_file
            let write_call = ToolCall {
                id: "call-1".into(),
                name: "write_file".into(),
                arguments: json!({
                    "path": "src/main.rs",
                    "content": "fn main() { println!(\"hello\"); }\n"
                })
                .to_string(),
            };
            assert_eq!(
                executor.describe(&write_call),
                "write src/main.rs (1 lines, 33 B)"
            );
            let outcome = executor.execute(&write_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "wrote src/main.rs");
            assert!(outcome.diff.is_some());
            assert_eq!(outcome.diff.unwrap().old, None);

            // writing inside .git is refused; VFS remains unchanged
            for git_path in &[".git/hooks/pre-commit", "sub/.git/config", ".GIT/config"] {
                let call = ToolCall {
                    id: "call-git-refuse".into(),
                    name: "write_file".into(),
                    arguments: json!({
                        "path": git_path,
                        "content": "#!/bin/sh\nexit 1\n"
                    })
                    .to_string(),
                };
                let outcome = executor.execute(&call).await;
                assert!(!outcome.ok);
                assert_eq!(outcome.content, "error: refusing to write inside .git");
                assert!(outcome.summary.contains("refusing to write inside .git"));
                assert!(executor.vfs().read(git_path).await.is_err());
            }

            // symlink traversal into .git is also refused; VFS remains unchanged
            executor.vfs().add_symlink("foo", ".git").unwrap();
            executor.vfs().add_symlink("sub/bar", "../.git").unwrap();
            executor
                .vfs()
                .add_symlink("gitlink", ".git/config")
                .unwrap();
            for symlink_path in &[
                "foo/config",
                "foo/hooks/pre-commit",
                "sub/bar/config",
                "gitlink",
            ] {
                let call = ToolCall {
                    id: "call-symlink-refuse".into(),
                    name: "write_file".into(),
                    arguments: json!({
                        "path": symlink_path,
                        "content": "#!/bin/sh\nexit 1\n"
                    })
                    .to_string(),
                };
                let outcome = executor.execute(&call).await;
                assert!(!outcome.ok);
                assert_eq!(outcome.content, "error: refusing to write inside .git");
                assert!(outcome.summary.contains("refusing to write inside .git"));
                assert!(executor.vfs().read(symlink_path).await.is_err());
            }

            // writing .gitignore is allowed
            let gitignore_call = ToolCall {
                id: "call-gitignore".into(),
                name: "write_file".into(),
                arguments: json!({
                    "path": ".gitignore",
                    "content": "target/\n"
                })
                .to_string(),
            };
            assert_eq!(
                executor.describe(&gitignore_call),
                "write .gitignore (1 lines, 8 B)"
            );
            let outcome = executor.execute(&gitignore_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "wrote .gitignore");
            assert_eq!(
                executor.vfs().read(".gitignore").await.unwrap(),
                "target/\n"
            );

            // read_file
            let read_call = ToolCall {
                id: "call-2".into(),
                name: "read_file".into(),
                arguments: json!({ "path": "src/main.rs" }).to_string(),
            };
            let outcome = executor.execute(&read_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "read src/main.rs (1 lines)");
            assert_eq!(outcome.content, "fn main() { println!(\"hello\"); }\n");

            // list_dir
            let list_call = ToolCall {
                id: "call-3".into(),
                name: "list_dir".into(),
                arguments: json!({ "path": "" }).to_string(),
            };
            let outcome = executor.execute(&list_call).await;
            assert!(outcome.ok);
            assert!(outcome.content.contains("[DIR]  src/"));

            // grep_search
            let grep_call = ToolCall {
                id: "call-4".into(),
                name: "grep_search".into(),
                arguments: json!({ "query": "println" }).to_string(),
            };
            let outcome = executor.execute(&grep_call).await;
            assert!(outcome.ok);
            assert!(outcome.content.contains("src/main.rs:1: fn main()"));

            // path escape rejection
            let escape_call = ToolCall {
                id: "call-5".into(),
                name: "read_file".into(),
                arguments: json!({ "path": "../outside.txt" }).to_string(),
            };
            let outcome = executor.execute(&escape_call).await;
            assert!(!outcome.ok);
            assert!(outcome.summary.contains("escapes"));
        });
    }

    struct MockWebClient;
    impl WebClient for MockWebClient {
        async fn search(&self, query: &str, _limit: usize) -> Result<Vec<WebSearchResult>, String> {
            if query == "error" {
                return Err("network timeout".into());
            }
            Ok(vec![WebSearchResult {
                title: "Rust Lang".into(),
                url: "https://www.rust-lang.org".into(),
                snippet: "A language empowering everyone to build reliable and efficient software."
                    .into(),
            }])
        }

        async fn fetch_page(&self, url: &str) -> Result<String, String> {
            if url.contains("error") {
                return Err("404 not found".into());
            }
            Ok("# Rust Documentation\n\nWelcome to Rust.".into())
        }
    }

    #[test]
    fn test_vfs_tool_executor_web_tools() {
        futures::executor::block_on(async {
            let vfs = MemoryVfs::new();
            let executor = VfsToolExecutor::with_web(vfs, MockWebClient);

            // search_web
            let search_call = ToolCall {
                id: "call-w1".into(),
                name: "search_web".into(),
                arguments: json!({ "query": "rust" }).to_string(),
            };
            let outcome = executor.execute(&search_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "search web 'rust' (1 results)");
            assert!(outcome.content.contains("Title: Rust Lang"));
            assert!(outcome.content.contains("https://www.rust-lang.org"));

            // fetch_web_page
            let fetch_call = ToolCall {
                id: "call-w2".into(),
                name: "fetch_web_page".into(),
                arguments: json!({ "url": "https://doc.rust-lang.org" }).to_string(),
            };
            let outcome = executor.execute(&fetch_call).await;
            assert!(outcome.ok);
            assert!(
                outcome
                    .summary
                    .starts_with("fetch https://doc.rust-lang.org")
            );
            assert!(outcome.content.contains("# Rust Documentation"));

            // invalid url error
            let bad_url_call = ToolCall {
                id: "call-w3".into(),
                name: "fetch_web_page".into(),
                arguments: json!({ "url": "ftp://bad-scheme" }).to_string(),
            };
            let outcome = executor.execute(&bad_url_call).await;
            assert!(!outcome.ok);
            assert!(
                outcome
                    .summary
                    .contains("must start with http:// or https://")
            );
        });
    }

    struct MockBridgeClient;
    impl BridgeClient for MockBridgeClient {
        async fn execute_command(
            &self,
            command: &str,
            _timeout_seconds: u64,
        ) -> Result<CommandOutcome, String> {
            if command == "fail" {
                return Err("command execution failed".into());
            }
            if command == "cargo test" {
                return Ok(CommandOutcome {
                    exit_code: Some(0),
                    stdout: "running 1 test\ntest test_ok ... ok\n".into(),
                    stderr: String::new(),
                });
            }
            Ok(CommandOutcome {
                exit_code: Some(1),
                stdout: String::new(),
                stderr: "command not found\n".into(),
            })
        }

        async fn git_status(&self) -> Result<GitRepoStatus, String> {
            let mut files = std::collections::HashMap::new();
            files.insert(
                "src/lib.rs".into(),
                openwebide_core::GitFileStatus::Modified,
            );
            Ok(GitRepoStatus {
                branch: "main".into(),
                commit_hash: "abcdef123456".into(),
                commit_message: Some("init".into()),
                upstream: Some("origin/main".into()),
                ahead: 1,
                behind: 0,
                is_clean: false,
                line_stats: openwebide_core::GitLineStats {
                    insertions: 5,
                    deletions: 2,
                },
                files,
            })
        }

        async fn git_diff(&self, path: Option<&str>) -> Result<String, String> {
            if let Some(p) = path {
                Ok(format!("diff --git a/{p} b/{p}\n+new line"))
            } else {
                Ok("diff --git a/src/lib.rs b/src/lib.rs\n+new line".into())
            }
        }

        async fn git_commit(&self, req: &GitCommitRequest) -> Result<GitCommitResult, String> {
            if req.message.contains("fail") {
                return Err("commit hook failed".into());
            }
            if matches!(req.paths.as_deref(), Some([])) {
                return Err("drift: paths is Some([]) instead of None".into());
            }
            Ok(GitCommitResult {
                commit_hash: "1234567890ab".into(),
                summary: format!("[main 1234567] {}", req.message),
                pre_commit_output: None,
                is_signed: true,
            })
        }

        async fn git_checkout(
            &self,
            req: &GitCheckoutRequest,
        ) -> Result<GitCheckoutResult, String> {
            Ok(GitCheckoutResult {
                branch: req.branch.clone(),
                previous_branch: Some("main".into()),
                switched: true,
            })
        }
    }

    #[test]
    fn test_vfs_tool_executor_run_command() {
        futures::executor::block_on(async {
            let vfs = MemoryVfs::new();
            let executor =
                VfsToolExecutor::with_web_and_bridge(vfs, MockWebClient, MockBridgeClient);

            // successful command
            let cmd_call = ToolCall {
                id: "call-cmd-1".into(),
                name: "run_command".into(),
                arguments: json!({ "command": "cargo test" }).to_string(),
            };
            let outcome = executor.execute(&cmd_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "ran 'cargo test' (exit 0)");
            assert!(outcome.content.contains("running 1 test"));

            // command with non-zero exit code
            let cmd_fail_call = ToolCall {
                id: "call-cmd-2".into(),
                name: "run_command".into(),
                arguments: json!({ "command": "unknown-cmd" }).to_string(),
            };
            let outcome = executor.execute(&cmd_fail_call).await;
            assert!(!outcome.ok);
            assert_eq!(outcome.summary, "ran 'unknown-cmd' (exit 1)");
            assert!(outcome.content.contains("Exit code: 1"));

            // bridge error
            let err_call = ToolCall {
                id: "call-cmd-3".into(),
                name: "run_command".into(),
                arguments: json!({ "command": "fail" }).to_string(),
            };
            let outcome = executor.execute(&err_call).await;
            assert!(!outcome.ok);
            assert!(outcome.summary.contains("command execution failed"));
        });
    }

    #[test]
    fn test_vfs_tool_executor_git_tools() {
        futures::executor::block_on(async {
            let vfs = MemoryVfs::new();
            let executor =
                VfsToolExecutor::with_web_and_bridge(vfs, MockWebClient, MockBridgeClient);

            // git_status
            let status_call = ToolCall {
                id: "call-git-1".into(),
                name: "git_status".into(),
                arguments: "{}".into(),
            };
            assert_eq!(executor.describe(&status_call), "inspect git status");
            let outcome = executor.execute(&status_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "git status: branch main, 1 changed files");
            assert!(outcome.content.contains("Branch: main"));
            assert!(outcome.content.contains("Changes: +5 -2"));
            assert!(outcome.content.contains("M src/lib.rs"));

            // git_diff (repo)
            let diff_call = ToolCall {
                id: "call-git-2".into(),
                name: "git_diff".into(),
                arguments: "{}".into(),
            };
            assert_eq!(executor.describe(&diff_call), "inspect repository git diff");
            let outcome = executor.execute(&diff_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "git diff (repository)");
            assert!(outcome.content.contains("diff --git a/src/lib.rs"));

            // git_diff (path)
            let diff_path_call = ToolCall {
                id: "call-git-3".into(),
                name: "git_diff".into(),
                arguments: json!({ "path": "src/main.rs" }).to_string(),
            };
            assert_eq!(
                executor.describe(&diff_path_call),
                "inspect git diff for 'src/main.rs'"
            );
            let outcome = executor.execute(&diff_path_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "git diff 'src/main.rs'");
            assert!(outcome.content.contains("diff --git a/src/main.rs"));

            // git_commit
            let commit_call = ToolCall {
                id: "call-git-4".into(),
                name: "git_commit".into(),
                arguments: json!({ "message": "feat: add feature" }).to_string(),
            };
            assert_eq!(
                executor.describe(&commit_call),
                "commit ALL tracked changes: 'feat: add feature'"
            );
            let commit_paths_call = ToolCall {
                id: "call-git-4-paths".into(),
                name: "git_commit".into(),
                arguments: json!({
                    "message": "feat: add feature",
                    "paths": ["src/main.rs", "Cargo.toml"]
                })
                .to_string(),
            };
            assert_eq!(
                executor.describe(&commit_paths_call),
                "commit src/main.rs, Cargo.toml: 'feat: add feature'"
            );
            let outcome = executor.execute(&commit_paths_call).await;
            assert!(outcome.ok);

            // git_commit with empty paths array [] drifts neither in describe() nor execution
            let commit_empty_paths_call = ToolCall {
                id: "call-git-4-empty".into(),
                name: "git_commit".into(),
                arguments: json!({
                    "message": "feat: add feature",
                    "paths": []
                })
                .to_string(),
            };
            assert_eq!(
                executor.describe(&commit_empty_paths_call),
                "commit ALL tracked changes: 'feat: add feature'"
            );
            let outcome = executor.execute(&commit_empty_paths_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "committed 1234567: 'feat: add feature'");

            let outcome = executor.execute(&commit_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "committed 1234567: 'feat: add feature'");
            assert!(outcome.content.contains("Signed: true"));

            // git_commit failure
            let commit_fail = ToolCall {
                id: "call-git-5".into(),
                name: "git_commit".into(),
                arguments: json!({ "message": "fail commit" }).to_string(),
            };
            let outcome = executor.execute(&commit_fail).await;
            assert!(!outcome.ok);
            assert!(outcome.summary.contains("commit hook failed"));

            // git_branch
            let branch_call = ToolCall {
                id: "call-git-6".into(),
                name: "git_branch".into(),
                arguments: json!({ "branch_name": "feat/my-feature" }).to_string(),
            };
            assert_eq!(
                executor.describe(&branch_call),
                "switch to branch 'feat/my-feature'"
            );
            let outcome = executor.execute(&branch_call).await;
            assert!(outcome.ok);
            assert_eq!(outcome.summary, "switched to branch 'feat/my-feature'");
            assert!(outcome.content.contains("Previous branch: main"));
        });
    }
}
