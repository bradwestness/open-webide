//! Shared domain types used across the Open WebIDE frontend, backend, and crates.

use serde::{Deserialize, Serialize};

/// A local-LLM runtime the IDE can talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Ollama,
    LlamaCpp,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LlamaCpp => "llamacpp",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ollama" => Some(Self::Ollama),
            "llamacpp" => Some(Self::LlamaCpp),
            _ => None,
        }
    }
}

/// A saved connection to a local-LLM runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// Set by the server; clients may omit it on update requests.
    #[serde(default)]
    pub id: i64,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: Option<String>,
    pub enabled: bool,
}

/// Payload for creating a new connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewConnection {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: Option<String>,
}

/// Role of a chat message.
///
/// `Tool` is a transient role used only inside the agent loop to carry a
/// tool result back to the model; it is never persisted (the `messages`
/// table rejects it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

/// A single chat message within a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Set by the server on insert; clients may omit it in requests.
    #[serde(default)]
    pub id: i64,
    /// Set by the server on insert; clients may omit it in requests.
    #[serde(default)]
    pub session_id: i64,
    pub role: Role,
    pub content: String,
    /// Set by the server on insert; clients may omit it in requests.
    #[serde(default)]
    pub created_at: i64,
    /// Tool calls this assistant message requested. Transient: set only in
    /// the in-memory agent loop, never persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// For `role = Tool`: the id of the tool call this result answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// A chat session bound to an optional LLM connection and project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: i64,
    pub name: String,
    pub connection_id: Option<i64>,
    /// Optional system prompt attached to the session.
    #[serde(default)]
    pub system_prompt_id: Option<i64>,
    /// The project this session belongs to.
    #[serde(default)]
    pub project_id: Option<i64>,
    pub created_at: i64,
}

/// Payload for creating a new chat session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSession {
    pub name: String,
    pub connection_id: Option<i64>,
    pub system_prompt_id: Option<i64>,
    #[serde(default)]
    pub project_id: Option<i64>,
}

/// A named system prompt the user can attach to a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemPrompt {
    pub id: i64,
    pub name: String,
    pub content: String,
}

/// A model reported by an LLM provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
}

/// Request to run a chat completion against a saved connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub connection_id: i64,
    pub system_prompt: Option<String>,
    /// Model override; falls back to the connection's model when absent.
    #[serde(default)]
    pub model: Option<String>,
    pub messages: Vec<ChatMessage>,
    /// Tools offered to the model; empty disables tool calling.
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

/// A tool the agent may call, described by a name and a JSON-schema
/// parameter object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema object describing the tool's arguments.
    pub parameters: serde_json::Value,
}

/// A tool call requested by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The call's arguments, encoded as a JSON string.
    pub arguments: String,
}

/// The outcome of a tool-capable chat completion: either the model's text
/// reply or the tool calls it wants to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

/// A file edit produced by the agent, for diff rendering in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    /// Previous contents; `None` when the file is new.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    pub new: String,
}

/// Health/status payload returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub status: String,
    pub version: String,
}

/// Where a project's files live.
///
/// `Remote` = on the machine running Spin (the backend reads/writes them via
/// the WASI filesystem). `Local` = on the machine running the browser (the
/// frontend reads/writes them via the File System Access API).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceMode {
    Remote,
    Local,
}

impl WorkspaceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Local => "local",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "remote" => Some(Self::Remote),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
}

/// A project: a named folder the IDE operates on, plus its workspace mode.
/// Sessions belong to a project; a project is a first-class entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub mode: WorkspaceMode,
    /// The folder path. For `Remote` mode, a path relative to the mounted
    /// workspace root. For `Local` mode, unused (the browser holds the
    /// directory handle).
    #[serde(default)]
    pub path: Option<String>,
    pub created_at: i64,
}

/// Payload for creating a new project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProject {
    pub name: String,
    pub mode: WorkspaceMode,
    #[serde(default)]
    pub path: Option<String>,
}

/// A single entry in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    /// Path relative to the workspace root.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}
