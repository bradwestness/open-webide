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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
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
}

/// A chat session bound to an optional LLM connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: i64,
    pub name: String,
    pub connection_id: Option<i64>,
    pub created_at: i64,
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
}

/// Health/status payload returned by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub status: String,
    pub version: String,
}
