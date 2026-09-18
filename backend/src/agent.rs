//! Agentic coding for remote-mode projects: the backend's `ToolExecutor`
//! (workspace-confined file tools) and the SSE stream that wraps the agent
//! loop from the `openwebide-agent` crate.

use std::pin::Pin;

use futures::{Stream, StreamExt, stream};
use openwebide_agent::{AgentConfig, AgentEvent, ToolExecutor, ToolOutcome};
use openwebide_core::{ChatMessage, ChatRequest, FileDiff, Role, ToolCall, ToolDefinition};
use openwebide_llm::registry::Provider;
use openwebide_storage::{Store, spin_db::SpinDb};
use serde_json::{Value, json};

use crate::http_client::SpinHttpClient;
use crate::sse::SseEvent;
use crate::state::now;

/// A tool executor bound to one project's workspace. Every tool path is
/// resolved against the project's base directory (itself relative to the
/// Spin preopen root), so the agent can only touch files inside the project.
pub struct WorkspaceExecutor {
    base: String,
}

/// The workspace tools offered to the model.
pub fn workspace_tools() -> Vec<ToolDefinition> {
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
            description:
                "Create or overwrite a file in the workspace with the given full contents.".into(),
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
                    "path": { "type": "string", "description": "Workspace-relative directory path (empty for the root)" }
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
                    "path": { "type": "string", "description": "Workspace-relative directory to search in (empty for the root)" }
                },
                "required": ["query"]
            }),
        },
    ]
}

impl ToolExecutor for WorkspaceExecutor {
    fn describe(&self, call: &ToolCall) -> String {
        let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
        match call.name.as_str() {
            "read_file" => format!("read {}", arg_path(&args)),
            "write_file" => format!("write {}", arg_path(&args)),
            "list_dir" => format!("list {}", arg_path_or_root(&args)),
            "search" => format!(
                "search '{}'",
                args.get("query").and_then(|v| v.as_str()).unwrap_or("")
            ),
            other => other.to_string(),
        }
    }

    async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
        match call.name.as_str() {
            "read_file" => self.read_file(&args).await,
            "write_file" => self.write_file(&args).await,
            "list_dir" => self.list_dir(&args).await,
            "search" => self.search(&args).await,
            other => ToolOutcome {
                ok: false,
                content: format!("unknown tool: {other}"),
                summary: format!("unknown tool: {other}"),
                diff: None,
            },
        }
    }
}

impl WorkspaceExecutor {
    /// Join the project base with a tool-supplied relative path, rejecting
    /// anything that would escape the project. The files layer sanitizes the
    /// result a second time; this keeps the error message clear.
    fn resolve(&self, rel: &str) -> Result<String, String> {
        if rel.starts_with('/') || rel.split('/').any(|c| c == "..") {
            return Err(format!("path {rel:?} escapes the workspace"));
        }
        if self.base.is_empty() {
            Ok(rel.to_string())
        } else {
            Ok(format!(
                "{}/{}",
                self.base.trim_end_matches('/'),
                rel.trim_start_matches('/')
            ))
        }
    }

    async fn read_file(&self, args: &Value) -> ToolOutcome {
        let rel = arg_path(args);
        match self.resolve(&rel) {
            Err(e) => fail("read_file", &rel, &e),
            Ok(full) => match crate::files::read(&full).await {
                Ok(content) => {
                    let lines = content.lines().count();
                    ToolOutcome {
                        ok: true,
                        content,
                        summary: format!("read {rel} ({lines} lines)"),
                        diff: None,
                    }
                }
                Err(e) => fail("read_file", &rel, &e.to_string()),
            },
        }
    }

    async fn write_file(&self, args: &Value) -> ToolOutcome {
        let rel = arg_path(args);
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        match self.resolve(&rel) {
            Err(e) => fail("write_file", &rel, &e),
            Ok(full) => {
                // Capture the previous contents for the diff (None if new).
                let old = crate::files::read(&full).await.ok();
                match crate::files::write(&full, &content).await {
                    Ok(()) => ToolOutcome {
                        ok: true,
                        content: format!("wrote {rel} ({} bytes)", content.len()),
                        summary: format!("wrote {rel}"),
                        diff: Some(FileDiff {
                            path: rel,
                            old,
                            new: content,
                        }),
                    },
                    Err(e) => fail("write_file", &rel, &e.to_string()),
                }
            }
        }
    }

    async fn list_dir(&self, args: &Value) -> ToolOutcome {
        let rel = arg_path(args);
        let display = if rel.is_empty() {
            ".".to_string()
        } else {
            rel.clone()
        };
        match self.resolve(&rel) {
            Err(e) => fail("list_dir", &display, &e),
            Ok(full) => match crate::files::list(&full).await {
                Ok(entries) => {
                    let content = if entries.is_empty() {
                        "(empty)".to_string()
                    } else {
                        entries
                            .iter()
                            .map(|e| format!("{} {}", if e.is_dir { "d" } else { "f" }, e.name))
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    ToolOutcome {
                        ok: true,
                        content,
                        summary: format!("listed {display} ({} entries)", entries.len()),
                        diff: None,
                    }
                }
                Err(e) => fail("list_dir", &display, &e.to_string()),
            },
        }
    }

    async fn search(&self, args: &Value) -> ToolOutcome {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let rel = arg_path(args);
        match self.resolve(&rel) {
            Err(e) => fail("search", &query, &e),
            Ok(full) => match crate::files::search(&full, &query).await {
                Ok(entries) => {
                    let content = if entries.is_empty() {
                        "(no matches)".to_string()
                    } else {
                        entries
                            .iter()
                            .map(|e| e.path.clone())
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    ToolOutcome {
                        ok: true,
                        content,
                        summary: format!("searched '{query}' ({} matches)", entries.len()),
                        diff: None,
                    }
                }
                Err(e) => fail("search", &query, &e.to_string()),
            },
        }
    }
}

/// The `path` argument, or empty.
fn arg_path(args: &Value) -> String {
    args.get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// The `path` argument, or `.` when empty (for display).
fn arg_path_or_root(args: &Value) -> String {
    let p = arg_path(args);
    if p.is_empty() { ".".to_string() } else { p }
}

/// A failed tool outcome.
fn fail(name: &str, target: &str, message: &str) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        content: format!("{name} {target}: {message}"),
        summary: format!("failed to {name} {target}"),
        diff: None,
    }
}

/// Build the SSE event stream for one agentic message: the user message, the
/// agent's tool steps, and (on success) the persisted assistant message.
///
/// The store is moved in: the assistant message is persisted from inside the
/// stream, so the response body outlives the request handler.
pub fn agent_stream(
    store: Store<SpinDb>,
    session_id: i64,
    user_message: ChatMessage,
    request: ChatRequest,
    provider: Provider<SpinHttpClient>,
    base: String,
    config: AgentConfig,
) -> Pin<Box<dyn Stream<Item = SseEvent> + Send + 'static>> {
    let executor = WorkspaceExecutor { base };
    let events = openwebide_agent::run(provider, executor, request, config);
    let tail = stream::unfold((store, session_id, events), |state| async move {
        let (store, session_id, mut events) = state;
        let event = events.next().await?;
        let sse = match event {
            AgentEvent::ToolCall { id, name, summary } => SseEvent::ToolCall { id, name, summary },
            AgentEvent::ToolResult {
                id,
                name,
                ok,
                summary,
                diff,
            } => SseEvent::ToolResult {
                id,
                name,
                ok,
                summary,
                diff,
            },
            AgentEvent::FinalText(text) => match store
                .insert_message(session_id, Role::Assistant, &text, now())
                .await
            {
                Ok(message) => SseEvent::Done(message),
                Err(error) => SseEvent::Error(format!("failed to save reply: {error}")),
            },
            AgentEvent::Error(message) => SseEvent::Error(message),
        };
        Some((sse, (store, session_id, events)))
    });
    Box::pin(stream::iter([SseEvent::Message(user_message)]).chain(tail))
}
