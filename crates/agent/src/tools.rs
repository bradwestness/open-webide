//! Typed tool calls.
//!
//! A wire-level [`ToolCall`] is parsed into a [`Tool`] whose variants carry
//! typed argument structs, so malformed JSON arguments (a truncated streamed
//! call, a model typo) become a [`ToolArgError`] instead of silently running
//! a tool with empty defaults. [`ToolName`] is the single source of truth for
//! the tool set: its wire names, JSON schemas, and approval policy.

use std::fmt;
use std::str::FromStr;

use openwebide_core::{ToolCall, ToolDefinition};
use serde::Deserialize;
use serde_json::json;

/// Arguments for `read_file`.
#[derive(Debug, Clone, Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
    /// Forward-compatible fields: deserialized now, wired up in a later step.
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

/// Arguments for `write_file`.
#[derive(Debug, Clone, Deserialize)]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
}

/// Arguments for `list_dir`.
#[derive(Debug, Clone, Deserialize)]
pub struct ListDirArgs {
    pub path: Option<String>,
}

/// Arguments for `search`.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchArgs {
    pub query: String,
    pub path: Option<String>,
    /// Forward-compatible field: deserialized now, wired up in a later step.
    pub include_ignored: Option<bool>,
}

/// Arguments for `grep_search`.
#[derive(Debug, Clone, Deserialize)]
pub struct GrepSearchArgs {
    pub query: String,
    pub path: Option<String>,
    /// Forward-compatible field: deserialized now, wired up in a later step.
    pub include_ignored: Option<bool>,
}

/// Arguments for `search_web`.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchWebArgs {
    pub query: String,
    pub limit: Option<u64>,
}

/// Arguments for `fetch_web_page`.
#[derive(Debug, Clone, Deserialize)]
pub struct FetchWebPageArgs {
    pub url: String,
}

/// Arguments for `run_command`.
#[derive(Debug, Clone, Deserialize)]
pub struct RunCommandArgs {
    pub command: String,
    pub timeout_seconds: Option<u64>,
}

/// Arguments for `git_diff`.
#[derive(Debug, Clone, Deserialize)]
pub struct GitDiffArgs {
    pub path: Option<String>,
}

/// Arguments for `git_commit`.
#[derive(Debug, Clone, Deserialize)]
pub struct GitCommitArgs {
    pub message: String,
    pub paths: Option<Vec<String>>,
}

/// Arguments for `git_branch`.
#[derive(Debug, Clone, Deserialize)]
pub struct GitBranchArgs {
    pub branch_name: String,
    /// Forward-compatible field: deserialized now, wired up in a later step.
    pub create: Option<bool>,
}

/// A parsed tool call: the tool plus its typed arguments.
#[derive(Debug, Clone)]
pub enum Tool {
    ReadFile(ReadFileArgs),
    WriteFile(WriteFileArgs),
    ListDir(ListDirArgs),
    Search(SearchArgs),
    GrepSearch(GrepSearchArgs),
    SearchWeb(SearchWebArgs),
    FetchWebPage(FetchWebPageArgs),
    RunCommand(RunCommandArgs),
    GitStatus,
    GitDiff(GitDiffArgs),
    GitCommit(GitCommitArgs),
    GitBranch(GitBranchArgs),
}

impl Tool {
    /// A short human-readable description of what this call will do, shown
    /// in the UI before the tool runs.
    pub fn describe(&self) -> String {
        match self {
            Tool::ReadFile(args) => format!("read {}", args.path),
            Tool::WriteFile(args) => {
                let n = args.content.lines().count();
                let bytes = args.content.len();
                format!("write {} ({n} lines, {bytes} B)", args.path)
            }
            Tool::ListDir(args) => format!("list {}", args.path.as_deref().unwrap_or_default()),
            Tool::Search(args) => format!("search '{}'", args.query),
            Tool::GrepSearch(args) => format!("grep '{}'", args.query),
            Tool::SearchWeb(args) => format!("search web for '{}'", args.query),
            Tool::FetchWebPage(args) => format!("fetch web page {}", args.url),
            Tool::RunCommand(args) => format!("run '{}'", args.command),
            Tool::GitStatus => "inspect git status".to_string(),
            Tool::GitDiff(args) => match args.path.as_deref() {
                Some(p) => format!("inspect git diff for '{p}'"),
                None => "inspect repository git diff".to_string(),
            },
            Tool::GitCommit(args) => {
                let paths = args.paths.as_deref().filter(|p| !p.is_empty());
                match paths {
                    Some(p) => format!("commit {}: '{}'", p.join(", "), args.message),
                    None => format!("commit ALL tracked changes: '{}'", args.message),
                }
            }
            Tool::GitBranch(args) => format!("switch to branch '{}'", args.branch_name),
        }
    }
}

/// The name of a built-in tool, matching the wire names sent to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolName {
    ReadFile,
    WriteFile,
    ListDir,
    Search,
    GrepSearch,
    SearchWeb,
    FetchWebPage,
    RunCommand,
    GitStatus,
    GitDiff,
    GitCommit,
    GitBranch,
}

impl ToolName {
    /// Every built-in tool, in the order `vfs_tools()` advertises it to the
    /// model.
    pub const ALL: &[ToolName] = &[
        ToolName::ReadFile,
        ToolName::WriteFile,
        ToolName::ListDir,
        ToolName::Search,
        ToolName::GrepSearch,
        ToolName::SearchWeb,
        ToolName::FetchWebPage,
        ToolName::RunCommand,
        ToolName::GitStatus,
        ToolName::GitDiff,
        ToolName::GitCommit,
        ToolName::GitBranch,
    ];

    /// The wire name of the tool (e.g. `"read_file"`).
    pub fn as_str(self) -> &'static str {
        match self {
            ToolName::ReadFile => "read_file",
            ToolName::WriteFile => "write_file",
            ToolName::ListDir => "list_dir",
            ToolName::Search => "search",
            ToolName::GrepSearch => "grep_search",
            ToolName::SearchWeb => "search_web",
            ToolName::FetchWebPage => "fetch_web_page",
            ToolName::RunCommand => "run_command",
            ToolName::GitStatus => "git_status",
            ToolName::GitDiff => "git_diff",
            ToolName::GitCommit => "git_commit",
            ToolName::GitBranch => "git_branch",
        }
    }

    /// The tool's JSON-schema definition, as advertised to the model.
    pub fn definition(self) -> ToolDefinition {
        match self {
            ToolName::ReadFile => ToolDefinition {
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
            ToolName::WriteFile => ToolDefinition {
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
            ToolName::ListDir => ToolDefinition {
                name: "list_dir".into(),
                description: "List the entries of a directory in the workspace.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Workspace-relative directory path (empty for root)" }
                    }
                }),
            },
            ToolName::Search => ToolDefinition {
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
            ToolName::GrepSearch => ToolDefinition {
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
            ToolName::SearchWeb => ToolDefinition {
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
            ToolName::FetchWebPage => ToolDefinition {
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
            ToolName::RunCommand => ToolDefinition {
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
            ToolName::GitStatus => ToolDefinition {
                name: "git_status".into(),
                description: "Inspect uncommitted modifications, untracked files, and current branch status.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolName::GitDiff => ToolDefinition {
                name: "git_diff".into(),
                description: "View the unified diff of uncommitted changes in the repository or for a specific file.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Optional file path to inspect." }
                    }
                }),
            },
            ToolName::GitCommit => ToolDefinition {
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
            ToolName::GitBranch => ToolDefinition {
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
        }
    }

    /// Whether the tool needs explicit user approval before it runs.
    ///
    /// Read-only tools run automatically; everything else (writes, shell
    /// execution, git mutations, external fetches) is gated.
    pub fn requires_approval(self) -> bool {
        !matches!(
            self,
            ToolName::ReadFile
                | ToolName::ListDir
                | ToolName::Search
                | ToolName::GrepSearch
                | ToolName::GitStatus
                | ToolName::GitDiff
                | ToolName::SearchWeb
        )
    }

    /// Whether the tool may be auto-approved for the whole session.
    pub fn always_approvable(self) -> bool {
        !matches!(self, ToolName::RunCommand)
    }

    /// Whether the tool runs through the bridge daemon rather than the VFS.
    pub fn needs_bridge(self) -> bool {
        matches!(
            self,
            ToolName::RunCommand
                | ToolName::GitStatus
                | ToolName::GitDiff
                | ToolName::GitCommit
                | ToolName::GitBranch
        )
    }
}

/// Returned by [`ToolName::from_str`] for a name that is not a built-in tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownToolName;

impl FromStr for ToolName {
    type Err = UnknownToolName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read_file" => Ok(ToolName::ReadFile),
            "write_file" => Ok(ToolName::WriteFile),
            "list_dir" => Ok(ToolName::ListDir),
            "search" => Ok(ToolName::Search),
            "grep_search" => Ok(ToolName::GrepSearch),
            "search_web" => Ok(ToolName::SearchWeb),
            "fetch_web_page" => Ok(ToolName::FetchWebPage),
            "run_command" => Ok(ToolName::RunCommand),
            "git_status" => Ok(ToolName::GitStatus),
            "git_diff" => Ok(ToolName::GitDiff),
            "git_commit" => Ok(ToolName::GitCommit),
            "git_branch" => Ok(ToolName::GitBranch),
            _ => Err(UnknownToolName),
        }
    }
}

/// Why a tool call could not be parsed into a [`Tool`].
#[derive(Debug, Clone)]
pub enum ToolArgError {
    /// The call names a tool the executor does not implement.
    UnknownTool(String),
    /// The call's arguments are not valid JSON for the named tool.
    InvalidArguments { tool: &'static str, error: String },
}

impl fmt::Display for ToolArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolArgError::UnknownTool(name) => write!(f, "unknown tool: {name}"),
            ToolArgError::InvalidArguments { tool, error } => {
                write!(f, "invalid arguments for {tool}: {error}")
            }
        }
    }
}

/// Parse a wire-level tool call into a typed [`Tool`].
///
/// An unknown tool name is [`ToolArgError::UnknownTool`]. `git_status` takes
/// no arguments, so its `arguments` are ignored (even when malformed). An
/// empty `arguments` string is treated as `{}`; malformed JSON or wrong field
/// types are [`ToolArgError::InvalidArguments`].
pub fn parse(call: &ToolCall) -> Result<Tool, ToolArgError> {
    let name: ToolName = call
        .name
        .parse()
        .map_err(|_| ToolArgError::UnknownTool(call.name.clone()))?;
    if name == ToolName::GitStatus {
        // The git_status handler never looks at the arguments, so don't
        // fail a call just because its payload is empty or malformed.
        return Ok(Tool::GitStatus);
    }
    let raw = call.arguments.trim();
    let raw = if raw.is_empty() { "{}" } else { raw };
    macro_rules! parse_as {
        ($variant:ident, $ty:ty) => {
            serde_json::from_str::<$ty>(raw)
                .map(Tool::$variant)
                .map_err(|e| ToolArgError::InvalidArguments {
                    tool: name.as_str(),
                    error: e.to_string(),
                })
        };
    }
    match name {
        ToolName::ReadFile => parse_as!(ReadFile, ReadFileArgs),
        ToolName::WriteFile => parse_as!(WriteFile, WriteFileArgs),
        ToolName::ListDir => parse_as!(ListDir, ListDirArgs),
        ToolName::Search => parse_as!(Search, SearchArgs),
        ToolName::GrepSearch => parse_as!(GrepSearch, GrepSearchArgs),
        ToolName::SearchWeb => parse_as!(SearchWeb, SearchWebArgs),
        ToolName::FetchWebPage => parse_as!(FetchWebPage, FetchWebPageArgs),
        ToolName::RunCommand => parse_as!(RunCommand, RunCommandArgs),
        ToolName::GitStatus => unreachable!("handled above"),
        ToolName::GitDiff => parse_as!(GitDiff, GitDiffArgs),
        ToolName::GitCommit => parse_as!(GitCommit, GitCommitArgs),
        ToolName::GitBranch => parse_as!(GitBranch, GitBranchArgs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "call-1".into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    #[test]
    fn parse_each_tool_with_valid_args() {
        let cases = [
            (
                "read_file",
                r#"{"path": "src/main.rs"}"#,
                ToolName::ReadFile,
            ),
            (
                "write_file",
                r#"{"path": "src/main.rs", "content": "fn main() {}"}"#,
                ToolName::WriteFile,
            ),
            ("list_dir", r#"{"path": "src"}"#, ToolName::ListDir),
            ("list_dir", r#"{}"#, ToolName::ListDir),
            ("search", r#"{"query": "main"}"#, ToolName::Search),
            (
                "grep_search",
                r#"{"query": "fn main", "path": "src"}"#,
                ToolName::GrepSearch,
            ),
            (
                "search_web",
                r#"{"query": "rust wasm", "limit": 3}"#,
                ToolName::SearchWeb,
            ),
            (
                "fetch_web_page",
                r#"{"url": "https://doc.rust-lang.org"}"#,
                ToolName::FetchWebPage,
            ),
            (
                "run_command",
                r#"{"command": "cargo test", "timeout_seconds": 60}"#,
                ToolName::RunCommand,
            ),
            ("git_status", r#"{}"#, ToolName::GitStatus),
            ("git_diff", r#"{"path": "src/main.rs"}"#, ToolName::GitDiff),
            (
                "git_commit",
                r#"{"message": "feat: x", "paths": ["a.rs", "b.rs"]}"#,
                ToolName::GitCommit,
            ),
            (
                "git_branch",
                r#"{"branch_name": "feat/x"}"#,
                ToolName::GitBranch,
            ),
        ];
        for (name, arguments, expected) in cases {
            let tool = parse(&call(name, arguments))
                .unwrap_or_else(|e| panic!("parse({name}, {arguments}) failed: {e}"));
            assert_eq!(
                tool_name_of(&tool),
                expected,
                "parse({name}) matched the wrong variant"
            );
        }
    }

    fn tool_name_of(tool: &Tool) -> ToolName {
        match tool {
            Tool::ReadFile(_) => ToolName::ReadFile,
            Tool::WriteFile(_) => ToolName::WriteFile,
            Tool::ListDir(_) => ToolName::ListDir,
            Tool::Search(_) => ToolName::Search,
            Tool::GrepSearch(_) => ToolName::GrepSearch,
            Tool::SearchWeb(_) => ToolName::SearchWeb,
            Tool::FetchWebPage(_) => ToolName::FetchWebPage,
            Tool::RunCommand(_) => ToolName::RunCommand,
            Tool::GitStatus => ToolName::GitStatus,
            Tool::GitDiff(_) => ToolName::GitDiff,
            Tool::GitCommit(_) => ToolName::GitCommit,
            Tool::GitBranch(_) => ToolName::GitBranch,
        }
    }

    #[test]
    fn parse_write_file_wrong_type_reports_the_mismatch() {
        let err = parse(&call("write_file", r#"{"path": 3}"#)).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.starts_with("invalid arguments for write_file"),
            "unexpected message: {msg}"
        );
        // serde_json's Display reports the type mismatch itself; it does not
        // include the field name for value-level errors.
        assert!(
            msg.contains("invalid type: integer `3`, expected a string"),
            "unexpected message: {msg}"
        );
        match err {
            ToolArgError::InvalidArguments { tool, .. } => assert_eq!(tool, "write_file"),
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[test]
    fn parse_truncated_json_is_invalid_arguments() {
        let err = parse(&call("write_file", r#"{"path":"a","con"#)).unwrap_err();
        match err {
            ToolArgError::InvalidArguments { tool, .. } => assert_eq!(tool, "write_file"),
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[test]
    fn parse_git_status_ignores_arguments() {
        assert!(matches!(
            parse(&call("git_status", "")).unwrap(),
            Tool::GitStatus
        ));
        // Even malformed arguments don't matter: git_status takes none.
        assert!(matches!(
            parse(&call("git_status", r#"{"path":"a","con"#)).unwrap(),
            Tool::GitStatus
        ));
    }

    #[test]
    fn parse_unknown_tool_name() {
        let err = parse(&call("rm_rf", r#"{}"#)).unwrap_err();
        assert_eq!(err.to_string(), "unknown tool: rm_rf");
        match err {
            ToolArgError::UnknownTool(name) => assert_eq!(name, "rm_rf"),
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    #[test]
    fn parse_ignores_extra_fields() {
        let tool = parse(&call(
            "read_file",
            r#"{"path": "src/main.rs", "stray_field": 42}"#,
        ))
        .unwrap();
        match tool {
            Tool::ReadFile(args) => assert_eq!(args.path, "src/main.rs"),
            other => panic!("expected ReadFile, got {other:?}"),
        }
    }

    #[test]
    fn parse_empty_arguments_become_empty_object() {
        // A tool with optional-only fields parses fine from an empty payload.
        let tool = parse(&call("list_dir", "  ")).unwrap();
        match tool {
            Tool::ListDir(args) => assert!(args.path.is_none()),
            other => panic!("expected ListDir, got {other:?}"),
        }
    }

    #[test]
    fn tool_name_round_trips_through_wire_names() {
        for name in ToolName::ALL {
            assert_eq!(name.as_str().parse::<ToolName>().unwrap(), *name);
        }
        assert!(matches!(
            "not_a_tool".parse::<ToolName>(),
            Err(UnknownToolName)
        ));
    }

    #[test]
    fn tool_name_policy_methods_match_the_lists() {
        let auto: &[ToolName] = &[
            ToolName::ReadFile,
            ToolName::ListDir,
            ToolName::Search,
            ToolName::GrepSearch,
            ToolName::GitStatus,
            ToolName::GitDiff,
            ToolName::SearchWeb,
        ];
        for name in ToolName::ALL {
            assert_eq!(
                !name.requires_approval(),
                auto.contains(name),
                "requires_approval drifted for {name:?}"
            );
            assert_eq!(
                name.always_approvable(),
                !matches!(name, ToolName::RunCommand),
                "always_approvable drifted for {name:?}"
            );
            assert_eq!(
                name.needs_bridge(),
                matches!(
                    name,
                    ToolName::RunCommand
                        | ToolName::GitStatus
                        | ToolName::GitDiff
                        | ToolName::GitCommit
                        | ToolName::GitBranch
                ),
                "needs_bridge drifted for {name:?}"
            );
        }
    }

    #[test]
    fn describe_uses_typed_fields() {
        assert_eq!(
            parse(&call("read_file", r#"{"path": "src/main.rs"}"#))
                .unwrap()
                .describe(),
            "read src/main.rs"
        );
        assert_eq!(
            parse(&call(
                "write_file",
                r#"{"path": "src/main.rs", "content": "a\nb\n"}"#
            ))
            .unwrap()
            .describe(),
            "write src/main.rs (2 lines, 4 B)"
        );
        assert_eq!(
            parse(&call("list_dir", r#"{}"#)).unwrap().describe(),
            "list "
        );
        assert_eq!(
            parse(&call("git_commit", r#"{"message": "feat: x"}"#))
                .unwrap()
                .describe(),
            "commit ALL tracked changes: 'feat: x'"
        );
        assert_eq!(
            parse(&call(
                "git_commit",
                r#"{"message": "feat: x", "paths": ["a.rs", "b.rs"]}"#
            ))
            .unwrap()
            .describe(),
            "commit a.rs, b.rs: 'feat: x'"
        );
        // An empty paths array still reads as "commit ALL".
        assert_eq!(
            parse(&call(
                "git_commit",
                r#"{"message": "feat: x", "paths": []}"#
            ))
            .unwrap()
            .describe(),
            "commit ALL tracked changes: 'feat: x'"
        );
    }
}
