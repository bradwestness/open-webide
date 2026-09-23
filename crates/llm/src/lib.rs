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
use std::pin::Pin;

use bytes::Bytes;
use futures::Stream;
use openwebide_core::{
    ChatCompletion, ChatRequest, ModelInfo, ProviderKind, ToolDefinition, TurnTelemetry,
    estimate_chat_request_tokens, estimate_tokens,
};
use serde_json::json;

pub use error::ProviderError;

/// A chunk of a streamed chat completion.
pub enum StreamChunk {
    /// A content delta to append to the reply.
    Delta(String),
    /// The turn's usage, yielded exactly once, just before a successful end.
    Usage(TurnTelemetry),
}

/// `Some(Instant::now())` normally; `None` on `wasm32-unknown-unknown`, where
/// `Instant::now()` panics. Providers use this instead of calling
/// `Instant::now()` directly, so the same code compiles for the browser and
/// the timing there degrades to "unknown" (duration 0) rather than crashing.
pub(crate) fn clock_now() -> Option<std::time::Instant> {
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        None
    }
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        Some(std::time::Instant::now())
    }
}

/// Round a nanosecond duration to whole milliseconds, treating any positive
/// duration as at least 1ms (`eval_duration_ms == 0` means "unknown").
pub(crate) fn round_ns_to_ms(ns: u64) -> u64 {
    let ms = (ns + 500_000) / 1_000_000;
    if ns > 0 { ms.max(1) } else { 0 }
}

/// Accumulates a turn's usage as a provider response (or stream) is parsed,
/// applying the estimation fallback once the call finishes.
pub(crate) struct UsageAcc {
    pub prompt: Option<usize>,
    pub completion: Option<usize>,
    pub eval_ms: Option<u64>,
    /// Computed eagerly from the request, in case the provider omits
    /// `prompt_tokens`.
    pub prompt_estimate: usize,
    /// The reply text (or tool call name + arguments), for the completion
    /// estimate.
    pub text: String,
    pub started: Option<std::time::Instant>,
    pub ended: Option<std::time::Instant>,
}

impl UsageAcc {
    pub(crate) fn new(request: &ChatRequest) -> Self {
        Self {
            prompt: None,
            completion: None,
            eval_ms: None,
            prompt_estimate: estimate_chat_request_tokens(request),
            text: String::new(),
            started: None,
            ended: None,
        }
    }

    /// Apply the per-field fallbacks and produce the turn's telemetry.
    pub(crate) fn finish(&self) -> TurnTelemetry {
        let prompt_tokens = self.prompt.unwrap_or(self.prompt_estimate);
        let completion_tokens = self
            .completion
            .unwrap_or_else(|| estimate_tokens(&self.text));
        let eval_duration_ms = self
            .eval_ms
            .unwrap_or_else(|| match (self.started, self.ended) {
                (Some(start), Some(end)) => {
                    round_ns_to_ms(end.saturating_duration_since(start).as_nanos() as u64)
                }
                _ => 0,
            });
        TurnTelemetry {
            prompt_tokens,
            completion_tokens,
            eval_duration_ms,
            estimated: self.prompt.is_none() || self.completion.is_none(),
        }
    }
}

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

/// Provider wire format for the `tools` array, shared by both providers
/// (Ollama and the OpenAI-compatible llama.cpp API use the same shape).
pub(crate) fn tools_wire(tools: &[ToolDefinition]) -> serde_json::Value {
    serde_json::Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect(),
    )
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
    /// POST and stream the response body as byte chunks, yielding them as
    /// they arrive.
    fn post_stream(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send + 'static>>;
}

/// A local-LLM runtime the IDE can list models from and chat with.
pub trait LlmProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn list_models(&self) -> impl Future<Output = Result<Vec<ModelInfo>, ProviderError>> + Send;
    fn chat(
        &self,
        request: &ChatRequest,
    ) -> impl Future<Output = Result<String, ProviderError>> + Send;
    /// Stream a chat completion, yielding content deltas as they arrive, and
    /// exactly one [`StreamChunk::Usage`] just before a successful end.
    fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send + 'static>>;
    /// A tool-capable chat completion. Returns the model's text reply or the
    /// tool calls it wants to run (see [`ChatRequest::tools`]), plus the
    /// call's usage when the provider reported it.
    fn chat_tools(
        &self,
        request: &ChatRequest,
    ) -> impl Future<Output = Result<ChatCompletion, ProviderError>> + Send;
    /// The model's context window, in tokens, discovered from the runtime.
    /// `Ok(None)` when the runtime doesn't report one (or `model` is
    /// absent and the provider has no configured model either).
    fn context_limit(
        &self,
        model: Option<&str>,
    ) -> impl Future<Output = Result<Option<usize>, ProviderError>> + Send;
}

/// Extract a provider error from a stream line's `error` field, which may
/// be a plain string or an object with a `message` field.
pub(crate) fn stream_error(value: &serde_json::Value) -> Option<String> {
    match value.get("error")? {
        serde_json::Value::String(message) => Some(message.clone()),
        serde_json::Value::Object(_) => value["error"]
            .get("message")
            .and_then(|m| m.as_str())
            .map(str::to_string),
        _ => None,
    }
}

/// What one line of a provider stream means.
pub(crate) enum StreamLine {
    /// A content delta to emit.
    Delta(String),
    /// The stream finished successfully.
    Done,
    /// A keep-alive or metadata line to ignore.
    Skip,
}

/// Splits a byte-chunk stream into newline-delimited lines.
///
/// Handles chunks that split a line mid-way, `\r\n` line endings, and a
/// final line without a trailing newline.
pub(crate) struct LineStream<S> {
    inner: S,
    buffer: String,
}

impl<S> LineStream<S> {
    pub(crate) fn new(inner: S) -> Self {
        Self {
            inner,
            buffer: String::new(),
        }
    }
}

impl<S> Stream for LineStream<S>
where
    S: Stream<Item = Result<Bytes, ProviderError>> + Unpin,
{
    type Item = Result<String, ProviderError>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;

        let this = self.get_mut();
        loop {
            if let Some(pos) = this.buffer.find('\n') {
                let line = this.buffer[..pos].trim_end_matches('\r').to_string();
                this.buffer.drain(..=pos);
                if !line.is_empty() {
                    return Poll::Ready(Some(Ok(line)));
                }
                continue;
            }
            match Stream::poll_next(std::pin::Pin::new(&mut this.inner), cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    let text = String::from_utf8_lossy(&chunk);
                    this.buffer.push_str(&text);
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                Poll::Ready(None) => {
                    let line = std::mem::take(&mut this.buffer)
                        .trim_end_matches('\r')
                        .to_string();
                    if line.is_empty() {
                        return Poll::Ready(None);
                    }
                    return Poll::Ready(Some(Ok(line)));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
