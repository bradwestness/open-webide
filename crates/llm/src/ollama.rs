use openwebide_core::{ChatRequest, ModelInfo, ProviderKind};

use crate::{LlmProvider, ProviderError};

/// Provider for [Ollama](https://ollama.com).
///
/// Talks to the Ollama HTTP API (`/api/tags`, `/api/chat`).
pub struct OllamaProvider {
    base_url: String,
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }
}

impl LlmProvider for OllamaProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Ollama
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Err(ProviderError::NotImplemented(format!(
            "Ollama::list_models ({})",
            self.base_url
        )))
    }

    async fn chat(&self, _request: &ChatRequest) -> Result<String, ProviderError> {
        Err(ProviderError::NotImplemented(format!(
            "Ollama::chat ({})",
            self.base_url
        )))
    }
}
