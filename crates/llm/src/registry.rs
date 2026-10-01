use std::pin::Pin;

use futures::Stream;
use openwebide_core::{ChatCompletion, ChatRequest, Connection, ModelInfo, ProviderKind};

use crate::{
    HttpClient, LlmProvider, ProviderError, StreamChunk, ToolStreamChunk, ToolStreamMemo,
    llamacpp::LlamaCppProvider, ollama::OllamaProvider,
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
        Self::for_connection_with_memo(
            conn,
            http,
            ToolStreamMemo::new(conn.tool_stream_unsupported),
        )
    }

    pub fn for_connection_with_memo(conn: &Connection, http: C, memo: ToolStreamMemo) -> Self {
        match conn.kind {
            ProviderKind::Ollama => Self::Ollama(
                OllamaProvider::new(conn.base_url.clone(), conn.model.clone(), http)
                    .with_num_ctx(conn.context_limit),
            ),
            // llama.cpp has no per-request context size: `n_ctx` is fixed
            // when `llama-server` starts, so a configured limit only drives
            // the gauge, not the runtime window.
            ProviderKind::LlamaCpp => Self::LlamaCpp(
                LlamaCppProvider::new(conn.base_url.clone(), conn.model.clone(), http)
                    .with_tool_stream_memo(memo),
            ),
        }
    }
}

impl<C: HttpClient + 'static> LlmProvider for Provider<C> {
    fn tool_stream_memo(&self) -> Option<ToolStreamMemo> {
        match self {
            Self::LlamaCpp(provider) => provider.tool_stream_memo(),
            Self::Ollama(_) => None,
        }
    }

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
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send + 'static>> {
        match self {
            Self::Ollama(p) => p.chat_stream(request),
            Self::LlamaCpp(p) => p.chat_stream(request),
        }
    }

    async fn chat_tools(&self, request: &ChatRequest) -> Result<ChatCompletion, ProviderError> {
        match self {
            Self::Ollama(p) => p.chat_tools(request).await,
            Self::LlamaCpp(p) => p.chat_tools(request).await,
        }
    }

    fn chat_tools_stream(
        &self,
        request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<ToolStreamChunk, ProviderError>> + Send + 'static>> {
        match self {
            Self::Ollama(p) => p.chat_tools_stream(request),
            Self::LlamaCpp(p) => p.chat_tools_stream(request),
        }
    }

    async fn context_limit(&self, model: Option<&str>) -> Result<Option<usize>, ProviderError> {
        match self {
            Self::Ollama(p) => p.context_limit(model).await,
            Self::LlamaCpp(p) => p.context_limit(model).await,
        }
    }
}
