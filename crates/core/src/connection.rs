//! Shared connection domain types.

use serde::{Deserialize, Serialize};

/// A local-LLM runtime the IDE can talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Ollama,
    LlamaCpp,
}

impl ProviderKind {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LlamaCpp => "OpenAI-compatible",
        }
    }

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
    /// discovery, then [`crate::tui::DEFAULT_CONTEXT_LIMIT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<usize>,
    #[serde(default)]
    pub tool_stream_unsupported: bool,
    #[serde(default)]
    pub tool_stream_revision: i64,
    #[serde(default)]
    pub tool_selection: ToolSelection,
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

/// Server-scoped tools advertised to the model; host and model capabilities still apply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSelection {
    #[default]
    All,
    Selected(Vec<String>),
    ChatOnly,
}
impl ToolSelection {
    pub fn allows(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Selected(names) => names.iter().any(|selected| selected == name),
            Self::ChatOnly => false,
        }
    }
    pub fn apply(&self, tools: &mut Vec<crate::ToolDefinition>) {
        tools.retain(|tool| self.allows(&tool.name));
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Selected(names) = self {
            if names.len() > 128 {
                return Err("Select at most 128 tools.".into());
            }
            let mut seen = std::collections::HashSet::new();
            for name in names {
                if name.is_empty()
                    || name.len() > 128
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
                    })
                    || !seen.insert(name)
                {
                    return Err("Tool names must be unique and contain only letters, numbers, underscores, hyphens or dots.".into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_connections_keep_tools_and_invalid_selections_fail() {
        let connection: Connection = serde_json::from_value(serde_json::json!({"name":"test","kind":"ollama","base_url":"http://localhost","model":null,"enabled":true})).unwrap();
        assert_eq!(connection.tool_selection, ToolSelection::All);
        for names in [vec!["read_file", "read_file"], vec!["bad name"], vec![""]] {
            assert!(
                ToolSelection::Selected(names.into_iter().map(str::to_string).collect())
                    .validate()
                    .is_err()
            );
        }
        assert!(
            ToolSelection::Selected(vec!["future.tool".into()])
                .validate()
                .is_ok()
        );
        assert!(!ToolSelection::Selected(vec![]).allows("read_file"));
        assert_eq!(crate::context::tool_schema_tokens(&[]), 0);
    }
}
