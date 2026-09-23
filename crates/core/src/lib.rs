//! Shared domain types used across the Open WebIDE frontend, backend, and crates.

pub mod bridge;
pub mod file_type;
pub mod git;
pub mod highlight;
pub mod html;
pub mod tui;
pub mod vfs;

pub use bridge::*;
pub use file_type::*;
pub use git::*;
pub use html::html_to_markdown;
pub use tui::*;
pub use vfs::{MemoryVfs, Vfs, VfsError, VfsFuture, format_utc_timestamp, normalize_vfs_path};

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
    /// The model's context window, in tokens. `None` falls back to provider
    /// discovery, then [`tui::DEFAULT_CONTEXT_LIMIT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<usize>,
}

/// Payload for creating a new connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewConnection {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<usize>,
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
    /// Usage for the model call that produced this message, when the
    /// provider reported it. Persisted so a reloaded session's statusline
    /// and `/tokens` output match what a live session showed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnTelemetry>,
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
    /// The owning user, once accounts exist.
    #[serde(default)]
    pub user_id: Option<i64>,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

/// The wire body for `/api/chat-tools`: the completion plus the usage the
/// provider reported for the call, when it did.
///
/// `usage` is an `Option` so test fakes that only construct a `response`
/// stay valid; the real providers always return `Some`. This is not a wire
/// compatibility mechanism — frontend and backend ship together, and an
/// older bundle or backend does not deserialize this shape at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatCompletion {
    pub response: ChatResponse,
    #[serde(default)]
    pub usage: Option<TurnTelemetry>,
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

/// A web search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// A persisted agent tool step: the call and, once it finishes, its result.
///
/// Stored separately from chat messages (which feed the LLM context) so a
/// session's tool steps can be reloaded when switching tabs. The step renders
/// right after the user message that started its turn ([`ToolStep::anchor_message_id`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStep {
    /// The agent loop's step id for the call (matches the SSE
    /// `tool_call`/`tool_result` id), not the provider's; unique within the
    /// session. Rows from before step ids carry the provider's `call_N`.
    pub tool_call_id: String,
    /// The tool name (e.g. `write_file`).
    pub name: String,
    /// A human-readable description of the call (e.g. `write foo.txt`).
    pub summary: String,
    /// Whether the call succeeded; `None` while it is still running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// A one-line result summary; `None` while the call is still running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_summary: Option<String>,
    /// A file diff for edits; `None` for non-edit tools or while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<FileDiff>,
    /// The id of the user message that started this turn.
    pub anchor_message_id: i64,
}

/// One item in a session's persisted conversation, in the order it occurred:
/// a chat message or an agent tool step. Returned by the message-list
/// endpoint so a reloaded session shows its tool steps again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversationEntry {
    /// A user or assistant chat message.
    Message(ChatMessage),
    /// An agent tool step and, once it finished, its result.
    ToolStep(ToolStep),
}

/// A sub-line segment of diff text for intra-line highlighting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "text", rename_all = "snake_case")]
pub enum DiffChunk {
    /// Text common to both versions.
    Unchanged(String),
    /// Text deleted from the old version.
    Deleted(String),
    /// Text inserted into the new version.
    Inserted(String),
}

impl DiffChunk {
    pub fn text(&self) -> &str {
        match self {
            Self::Unchanged(s) | Self::Deleted(s) | Self::Inserted(s) => s,
        }
    }
}

/// A line in a diff with marker ('-', '+', ' ') and intra-line word chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub marker: char,
    pub content: String,
    pub chunks: Vec<DiffChunk>,
}

impl DiffLine {
    pub fn new_plain(marker: char, content: String) -> Self {
        let chunk = match marker {
            '-' => DiffChunk::Deleted(content.clone()),
            '+' => DiffChunk::Inserted(content.clone()),
            _ => DiffChunk::Unchanged(content.clone()),
        };
        Self {
            marker,
            content,
            chunks: vec![chunk],
        }
    }

    pub fn new_with_chunks(marker: char, content: String, chunks: Vec<DiffChunk>) -> Self {
        Self {
            marker,
            content,
            chunks,
        }
    }
}

/// Split a line into tokens (alphanumeric words, runs of whitespace, or punctuation symbols).
pub fn tokenize_diff_line(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        let is_alphanumeric = ch.is_alphanumeric() || ch == '_';
        let is_ws = ch.is_whitespace();
        let mut end = start + ch.len_utf8();
        while let Some(&(_, next_ch)) = chars.peek() {
            if (is_alphanumeric && (next_ch.is_alphanumeric() || next_ch == '_'))
                || (is_ws && next_ch.is_whitespace())
            {
                chars.next();
                end += next_ch.len_utf8();
            } else {
                break;
            }
        }
        tokens.push(&line[start..end]);
    }
    tokens
}

/// Compute intra-line word-level diff between two lines using LCS on tokens.
pub fn compute_word_diff(old_line: &str, new_line: &str) -> (Vec<DiffChunk>, Vec<DiffChunk>) {
    let old_tokens = tokenize_diff_line(old_line);
    let new_tokens = tokenize_diff_line(new_line);

    let m = old_tokens.len();
    let n = new_tokens.len();

    if m == 0 && n == 0 {
        return (Vec::new(), Vec::new());
    }
    if m == 0 {
        return (Vec::new(), vec![DiffChunk::Inserted(new_line.to_string())]);
    }
    if n == 0 {
        return (vec![DiffChunk::Deleted(old_line.to_string())], Vec::new());
    }

    // dp[i][j] stores length of LCS of old_tokens[..i] and new_tokens[..j]
    let mut dp = vec![vec![0u16; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            if old_tokens[i - 1] == new_tokens[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    // Backtrack to find matching tokens
    let mut old_matched = vec![false; m];
    let mut new_matched = vec![false; n];
    let mut i = m;
    let mut j = n;
    while i > 0 && j > 0 {
        if old_tokens[i - 1] == new_tokens[j - 1] {
            old_matched[i - 1] = true;
            new_matched[j - 1] = true;
            i -= 1;
            j -= 1;
        } else if dp[i - 1][j] >= dp[i][j - 1] {
            i -= 1;
        } else {
            j -= 1;
        }
    }

    // Build DiffChunks for old_line
    let mut old_chunks = Vec::new();
    let mut current_text = String::new();
    let mut current_is_match: Option<bool> = None;
    for (idx, &tok) in old_tokens.iter().enumerate() {
        let is_match = old_matched[idx];
        match current_is_match {
            Some(matched) if matched == is_match => {
                current_text.push_str(tok);
            }
            Some(matched) => {
                if matched {
                    old_chunks.push(DiffChunk::Unchanged(current_text));
                } else {
                    old_chunks.push(DiffChunk::Deleted(current_text));
                }
                current_text = tok.to_string();
                current_is_match = Some(is_match);
            }
            None => {
                current_text = tok.to_string();
                current_is_match = Some(is_match);
            }
        }
    }
    if let Some(matched) = current_is_match {
        if matched {
            old_chunks.push(DiffChunk::Unchanged(current_text));
        } else {
            old_chunks.push(DiffChunk::Deleted(current_text));
        }
    }

    // Build DiffChunks for new_line
    let mut new_chunks = Vec::new();
    let mut current_text = String::new();
    let mut current_is_match: Option<bool> = None;
    for (idx, &tok) in new_tokens.iter().enumerate() {
        let is_match = new_matched[idx];
        match current_is_match {
            Some(matched) if matched == is_match => {
                current_text.push_str(tok);
            }
            Some(matched) => {
                if matched {
                    new_chunks.push(DiffChunk::Unchanged(current_text));
                } else {
                    new_chunks.push(DiffChunk::Inserted(current_text));
                }
                current_text = tok.to_string();
                current_is_match = Some(is_match);
            }
            None => {
                current_text = tok.to_string();
                current_is_match = Some(is_match);
            }
        }
    }
    if let Some(matched) = current_is_match {
        if matched {
            new_chunks.push(DiffChunk::Unchanged(current_text));
        } else {
            new_chunks.push(DiffChunk::Inserted(current_text));
        }
    }

    (old_chunks, new_chunks)
}

/// Compute the changed middle of a file edit as inline `(marker, line)` pairs.
///
/// The common prefix and suffix lines are stripped; the result holds the
/// removed middle lines (marker `-`, from `old`) followed by the added middle
/// lines (marker `+`, from `new`). For a new file (`old` is `None`) every line
/// is `+`. Pure and natively unit-testable.
pub fn diff_inline_lines(diff: &FileDiff) -> Vec<(char, String)> {
    let old_lines: Vec<&str> = diff.old.as_deref().unwrap_or_default().lines().collect();
    let new_lines: Vec<&str> = diff.new.lines().collect();

    let mut i = 0;
    let mut j = 0;
    while i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
        i += 1;
        j += 1;
    }
    let mut old_end = old_lines.len();
    let mut new_end = new_lines.len();
    while old_end > i && new_end > j && old_lines[old_end - 1] == new_lines[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }

    let mut out = Vec::new();
    for line in &old_lines[i..old_end] {
        out.push(('-', line.to_string()));
    }
    for line in &new_lines[j..new_end] {
        out.push(('+', line.to_string()));
    }
    out
}

/// Compute the changed middle of a file edit as detailed `DiffLine`s with intra-line chunks.
pub fn diff_inline_detailed(diff: &FileDiff) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = diff.old.as_deref().unwrap_or_default().lines().collect();
    let new_lines: Vec<&str> = diff.new.lines().collect();

    let mut i = 0;
    let mut j = 0;
    while i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
        i += 1;
        j += 1;
    }
    let mut old_end = old_lines.len();
    let mut new_end = new_lines.len();
    while old_end > i && new_end > j && old_lines[old_end - 1] == new_lines[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }

    let old_mid = &old_lines[i..old_end];
    let new_mid = &new_lines[j..new_end];

    let mut del_lines = Vec::new();
    let mut add_lines = Vec::new();

    let min_len = old_mid.len().min(new_mid.len());
    for k in 0..min_len {
        let (old_chunks, new_chunks) = compute_word_diff(old_mid[k], new_mid[k]);
        del_lines.push(DiffLine::new_with_chunks(
            '-',
            old_mid[k].to_string(),
            old_chunks,
        ));
        add_lines.push(DiffLine::new_with_chunks(
            '+',
            new_mid[k].to_string(),
            new_chunks,
        ));
    }
    for line in old_mid.iter().skip(min_len) {
        del_lines.push(DiffLine::new_plain('-', line.to_string()));
    }
    for line in new_mid.iter().skip(min_len) {
        add_lines.push(DiffLine::new_plain('+', line.to_string()));
    }

    let mut out = del_lines;
    out.extend(add_lines);
    out
}

/// Compute the changed middle of a file edit as side-by-side `(old, new)` rows.
///
/// The common prefix and suffix lines appear on both sides. The changed middle
/// is aligned row-by-row, padding the shorter side with `None` (a pure
/// addition or removal). For a new file (`old` is `None`) every left cell is
/// `None`. Pure and natively unit-testable.
pub fn diff_side_by_side(diff: &FileDiff) -> Vec<(Option<String>, Option<String>)> {
    let old_lines: Vec<&str> = diff.old.as_deref().unwrap_or_default().lines().collect();
    let new_lines: Vec<&str> = diff.new.lines().collect();

    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_lines.len() - prefix
        && suffix < new_lines.len() - prefix
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut rows = Vec::new();
    for i in 0..prefix {
        rows.push((
            Some(old_lines[i].to_string()),
            Some(new_lines[i].to_string()),
        ));
    }
    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];
    for i in 0..old_mid.len().max(new_mid.len()) {
        rows.push((
            old_mid.get(i).map(|s| s.to_string()),
            new_mid.get(i).map(|s| s.to_string()),
        ));
    }
    for i in 0..suffix {
        rows.push((
            Some(old_lines[old_lines.len() - suffix + i].to_string()),
            Some(new_lines[new_lines.len() - suffix + i].to_string()),
        ));
    }
    rows
}

/// Compute the changed middle of a file edit as side-by-side rows with intra-line chunks.
pub fn diff_side_by_side_detailed(diff: &FileDiff) -> Vec<(Option<DiffLine>, Option<DiffLine>)> {
    let old_lines: Vec<&str> = diff.old.as_deref().unwrap_or_default().lines().collect();
    let new_lines: Vec<&str> = diff.new.lines().collect();

    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_lines.len() - prefix
        && suffix < new_lines.len() - prefix
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mut rows = Vec::new();
    for i in 0..prefix {
        rows.push((
            Some(DiffLine::new_plain(' ', old_lines[i].to_string())),
            Some(DiffLine::new_plain(' ', new_lines[i].to_string())),
        ));
    }
    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];

    let max_len = old_mid.len().max(new_mid.len());
    for i in 0..max_len {
        let old_item = old_mid.get(i);
        let new_item = new_mid.get(i);

        match (old_item, new_item) {
            (Some(&o), Some(&n)) => {
                let (old_chunks, new_chunks) = compute_word_diff(o, n);
                rows.push((
                    Some(DiffLine::new_with_chunks('-', o.to_string(), old_chunks)),
                    Some(DiffLine::new_with_chunks('+', n.to_string(), new_chunks)),
                ));
            }
            (Some(&o), None) => {
                rows.push((Some(DiffLine::new_plain('-', o.to_string())), None));
            }
            (None, Some(&n)) => {
                rows.push((None, Some(DiffLine::new_plain('+', n.to_string()))));
            }
            (None, None) => {}
        }
    }
    for i in 0..suffix {
        rows.push((
            Some(DiffLine::new_plain(
                ' ',
                old_lines[old_lines.len() - suffix + i].to_string(),
            )),
            Some(DiffLine::new_plain(
                ' ',
                new_lines[new_lines.len() - suffix + i].to_string(),
            )),
        ));
    }
    rows
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

/// Account role. `Admin` manages accounts; `User` is a regular local account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    User,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Admin => "admin",
            UserRole::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Self::Admin),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

/// A local user account. The password hash is internal to storage and is
/// never exposed through this type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: UserRole,
    pub created_at: i64,
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
    /// The owning user, once accounts exist. `None` for projects created
    /// before auth was introduced (reassigned to the first registered user).
    #[serde(default)]
    pub user_id: Option<i64>,
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

/// A single full-text search hit: one matching line in one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    /// Path relative to the project root.
    pub path: String,
    /// 1-based line number of the match.
    pub line: usize,
    /// The full text of the matching line.
    pub text: String,
}

/// Find every line in `content` that contains `query` (case-insensitive).
///
/// Returns `(1-based line number, line text)` pairs in file order. Pure and
/// allocation-light enough to run per-file on the backend, and natively
/// unit-testable in this crate.
pub fn find_content_matches(content: &str, query: &str) -> Vec<(usize, String)> {
    let q = query.to_lowercase();
    content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(&q))
        .map(|(i, line)| (i + 1, line.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_matches_finds_case_insensitive_lines() {
        let content = "fn main() {\n    let x = 1;\n    println!(\"Hello\");\n}\nfn MAIN() {}\n";
        let hits = find_content_matches(content, "main");
        assert_eq!(
            hits,
            vec![
                (1, "fn main() {".to_string()),
                (5, "fn MAIN() {}".to_string())
            ]
        );
    }

    #[test]
    fn content_matches_empty_query_matches_every_line() {
        let content = "a\nb\nc";
        assert_eq!(
            find_content_matches(content, ""),
            vec![(1, "a".into()), (2, "b".into()), (3, "c".into())]
        );
    }

    #[test]
    fn content_matches_no_match_is_empty() {
        assert!(find_content_matches("hello world", "zzz").is_empty());
    }

    #[test]
    fn diff_inline_strips_common_prefix_suffix() {
        let diff = FileDiff {
            path: "a.txt".into(),
            old: Some("a\nb\nc\nd".into()),
            new: "a\nX\nc\nd".into(),
        };
        assert_eq!(
            diff_inline_lines(&diff),
            vec![('-', "b".into()), ('+', "X".into())]
        );
    }

    #[test]
    fn diff_inline_new_file_is_all_additions() {
        let diff = FileDiff {
            path: "new.txt".into(),
            old: None,
            new: "a\nb".into(),
        };
        assert_eq!(
            diff_inline_lines(&diff),
            vec![('+', "a".into()), ('+', "b".into())]
        );
    }

    #[test]
    fn diff_inline_full_replacement_lists_all() {
        let diff = FileDiff {
            path: "a.txt".into(),
            old: Some("a\nb".into()),
            new: "x\ny".into(),
        };
        assert_eq!(
            diff_inline_lines(&diff),
            vec![
                ('-', "a".into()),
                ('-', "b".into()),
                ('+', "x".into()),
                ('+', "y".into())
            ]
        );
    }

    #[test]
    fn diff_side_by_side_aligns_changed_middle() {
        let diff = FileDiff {
            path: "a.txt".into(),
            old: Some("a\nb\nc".into()),
            new: "a\nx\nc".into(),
        };
        assert_eq!(
            diff_side_by_side(&diff),
            vec![
                (Some("a".into()), Some("a".into())),
                (Some("b".into()), Some("x".into())),
                (Some("c".into()), Some("c".into()))
            ]
        );
    }

    #[test]
    fn diff_side_by_side_new_file_pads_left() {
        let diff = FileDiff {
            path: "new.txt".into(),
            old: None,
            new: "a\nb".into(),
        };
        assert_eq!(
            diff_side_by_side(&diff),
            vec![(None, Some("a".into())), (None, Some("b".into()))]
        );
    }

    #[test]
    fn diff_side_by_side_deletion_pads_right() {
        let diff = FileDiff {
            path: "a.txt".into(),
            old: Some("a\nb\nc".into()),
            new: "a\nc".into(),
        };
        assert_eq!(
            diff_side_by_side(&diff),
            vec![
                (Some("a".into()), Some("a".into())),
                (Some("b".into()), None),
                (Some("c".into()), Some("c".into()))
            ]
        );
    }

    #[test]
    fn test_compute_word_diff_identifies_intra_line_changes() {
        let old = "let user_id = 42;";
        let new = "let account_id = 42;";
        let (old_chunks, new_chunks) = compute_word_diff(old, new);

        assert_eq!(
            old_chunks,
            vec![
                DiffChunk::Unchanged("let ".into()),
                DiffChunk::Deleted("user_id".into()),
                DiffChunk::Unchanged(" = 42;".into()),
            ]
        );

        assert_eq!(
            new_chunks,
            vec![
                DiffChunk::Unchanged("let ".into()),
                DiffChunk::Inserted("account_id".into()),
                DiffChunk::Unchanged(" = 42;".into()),
            ]
        );

        // Verification: reconstructing chunks reproduces original lines
        let reconstructed_old: String = old_chunks.iter().map(|c| c.text()).collect();
        let reconstructed_new: String = new_chunks.iter().map(|c| c.text()).collect();
        assert_eq!(reconstructed_old, old);
        assert_eq!(reconstructed_new, new);
    }

    #[test]
    fn test_diff_inline_detailed_with_word_chunks() {
        let diff = FileDiff {
            path: "test.rs".into(),
            old: Some("fn foo() -> i32 {\n    return 1;\n}\n".into()),
            new: "fn foo() -> i64 {\n    return 1;\n}\n".into(),
        };
        let detailed = diff_inline_detailed(&diff);
        assert_eq!(detailed.len(), 2);
        assert_eq!(detailed[0].marker, '-');
        assert_eq!(detailed[1].marker, '+');

        assert_eq!(
            detailed[0].chunks,
            vec![
                DiffChunk::Unchanged("fn foo() -> ".into()),
                DiffChunk::Deleted("i32".into()),
                DiffChunk::Unchanged(" {".into()),
            ]
        );
        assert_eq!(
            detailed[1].chunks,
            vec![
                DiffChunk::Unchanged("fn foo() -> ".into()),
                DiffChunk::Inserted("i64".into()),
                DiffChunk::Unchanged(" {".into()),
            ]
        );
    }
}
