use openwebide_core::{ChatRequest, ModelInfo, ProviderKind};

use crate::{LlmProvider, ProviderError};

/// Provider for a [llama.cpp](https://github.com/ggml-org/llama.cpp) server
/// (`llama-server`, OpenAI-compatible API).
pub struct LlamaCppProvider {
    base_url: String,
}

impl LlamaCppProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }
}

impl LlmProvider for LlamaCppProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::LlamaCpp
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Err(ProviderError::NotImplemented(format!(
            "LlamaCpp::list_models ({})",
            self.base_url
        )))
    }

    async fn chat(&self, _request: &ChatRequest) -> Result<String, ProviderError> {
        Err(ProviderError::NotImplemented(format!(
            "LlamaCpp::chat ({})",
            self.base_url
        )))
    }
}
