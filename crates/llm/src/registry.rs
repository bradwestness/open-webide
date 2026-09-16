use openwebide_core::{ChatRequest, Connection, ModelInfo, ProviderKind};

use crate::{LlmProvider, ProviderError, llamacpp::LlamaCppProvider, ollama::OllamaProvider};

/// The concrete provider set, selected by the connection's kind.
///
/// An enum rather than `Box<dyn LlmProvider>`: async trait methods are not
/// dyn-compatible, and the set of local-LLM runtimes is closed.
pub enum Provider {
    Ollama(OllamaProvider),
    LlamaCpp(LlamaCppProvider),
}

impl Provider {
    pub fn for_connection(conn: &Connection) -> Self {
        match conn.kind {
            ProviderKind::Ollama => Self::Ollama(OllamaProvider::new(conn.base_url.clone())),
            ProviderKind::LlamaCpp => Self::LlamaCpp(LlamaCppProvider::new(conn.base_url.clone())),
        }
    }
}

impl LlmProvider for Provider {
    fn kind(&self) -> ProviderKind {
        match self {
            Self::Ollama(p) => p.kind(),
            Self::LlamaCpp(p) => p.kind(),
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        match self {
            Self::Ollama(p) => p.list_models().await,
            Self::LlamaCpp(p) => p.list_models().await,
        }
    }

    async fn chat(&self, request: &ChatRequest) -> Result<String, ProviderError> {
        match self {
            Self::Ollama(p) => p.chat(request).await,
            Self::LlamaCpp(p) => p.chat(request).await,
        }
    }
}
