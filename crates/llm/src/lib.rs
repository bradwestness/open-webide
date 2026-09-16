//! LLM provider abstraction for Open WebIDE.
//!
//! Providers are constructed from a saved
//! [`Connection`](openwebide_core::Connection) via
//! [`registry::provider_for`].

pub mod error;
pub mod llamacpp;
pub mod ollama;
pub mod registry;

use std::future::Future;

use openwebide_core::{ChatRequest, ModelInfo, ProviderKind};

pub use error::ProviderError;

/// Minimal HTTP surface a provider needs to talk to a local-LLM runtime.
///
/// The host (the Spin backend) supplies the implementation; providers stay
/// transport-agnostic.
pub trait HttpClient: Send + Sync {
    fn get_json(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<serde_json::Value, ProviderError>> + Send;
    fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> impl Future<Output = Result<serde_json::Value, ProviderError>> + Send;
}

/// A local-LLM runtime the IDE can list models from and chat with.
pub trait LlmProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn list_models(&self) -> impl Future<Output = Result<Vec<ModelInfo>, ProviderError>> + Send;
    fn chat(
        &self,
        request: &ChatRequest,
    ) -> impl Future<Output = Result<String, ProviderError>> + Send;
}
