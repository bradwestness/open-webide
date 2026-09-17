//! LLM provider abstraction for Open WebIDE.
//!
//! Providers are constructed from a saved
//! [`Connection`](openwebide_core::Connection) via
//! [`registry::provider_for`].

pub mod error;
pub mod llamacpp;
pub mod ollama;
pub mod registry;

#[cfg(test)]
mod fake;

use std::future::Future;

use openwebide_core::{ChatRequest, ModelInfo, ProviderKind};
use serde_json::json;

pub use error::ProviderError;

/// Join a connection base URL and an API path, tolerating a trailing slash
/// on the base and a leading slash on the path.
pub(crate) fn url_for(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

/// Provider wire format for chat messages: `role` + `content` pairs, with
/// the system prompt first when one is set.
pub(crate) fn chat_messages(request: &ChatRequest) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();
    if let Some(system) = &request.system_prompt
        && !system.is_empty()
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for message in &request.messages {
        messages.push(json!({ "role": message.role.as_str(), "content": message.content }));
    }
    messages
}

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
