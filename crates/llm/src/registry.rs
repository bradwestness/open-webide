use std::pin::Pin;

use futures::Stream;
use openwebide_core::{ChatRequest, Connection, ModelInfo, ProviderKind};

use crate::{
    HttpClient, LlmProvider, ProviderError, llamacpp::LlamaCppProvider, ollama::OllamaProvider,
};

/// The concrete provider set, selected by the connection's kind.
///
/// An enum rather than `Box<dyn LlmProvider>`: async trait methods are not
/// dyn-compatible, and the set of local-LLM runtimes is closed.
pub enum Provider<C: HttpClient> {
    Ollama(OllamaProvider<C>),
    LlamaCpp(LlamaCppProvider<C>),
}

impl<C: HttpClient> Provider<C> {
    pub fn for_connection(conn: &Connection, http: C) -> Self {
        match conn.kind {
            ProviderKind::Ollama => Self::Ollama(OllamaProvider::new(
                conn.base_url.clone(),
                conn.model.clone(),
                http,
            )),
            ProviderKind::LlamaCpp => Self::LlamaCpp(LlamaCppProvider::new(
                conn.base_url.clone(),
                conn.model.clone(),
                http,
            )),
        }
    }
}

impl<C: HttpClient> LlmProvider for Provider<C> {
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

    fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send + 'static>> {
        match self {
            Self::Ollama(p) => p.chat_stream(request),
            Self::LlamaCpp(p) => p.chat_stream(request),
        }
    }
}
