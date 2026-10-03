//! LLM provider abstraction for Open WebIDE.
//!
//! Providers are constructed from a saved
//! [`Connection`](openwebide_core::Connection) via
//! [`registry::Provider::for_connection`].

pub mod discovery;
pub mod error;
pub mod llamacpp;
pub mod ollama;
pub mod registry;

pub(crate) mod sse;
mod streaming;
mod wire;

use streaming::{LineStream, StreamLine};
use wire::{chat_messages, tool_messages, tools_wire};

#[cfg(test)]
mod fake;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use bytes::Bytes;
use futures::Stream;
use openwebide_core::{
    ChatCompletion, ChatRequest, Connection, ModelInfo, ProviderKind, TurnTelemetry,
    estimate_chat_request_tokens, estimate_tokens,
};
use serde_json::json;

pub use error::ProviderError;
pub use openwebide_core::ToolStreamChunk;

pub fn completion_chunks(c: ChatCompletion) -> Vec<ToolStreamChunk> {
    let text = match &c.response {
        openwebide_core::ChatResponse::Text(text) => text,
        openwebide_core::ChatResponse::ToolCalls(_) => &c.preamble,
    };
    let mut chunks = Vec::new();
    if !c.reasoning.is_empty() {
        chunks.push(ToolStreamChunk::Reasoning(c.reasoning));
    }
    if !text.is_empty() {
        chunks.push(ToolStreamChunk::Delta(text.clone()));
    }
    chunks.push(ToolStreamChunk::Stop(c.stop_reason));
    if let Some(usage) = c.usage {
        chunks.push(ToolStreamChunk::Usage(usage));
    }
    chunks.push(ToolStreamChunk::Response(c.response));
    chunks
}

#[derive(Clone, Default)]
pub struct ToolStreamMemo {
    unsupported: Arc<AtomicBool>,
    recorded: Arc<AtomicBool>,
}

impl ToolStreamMemo {
    pub fn new(unsupported: bool) -> Self {
        Self {
            unsupported: Arc::new(AtomicBool::new(unsupported)),
            recorded: Arc::new(AtomicBool::new(unsupported)),
        }
    }

    pub fn unsupported(&self) -> bool {
        self.unsupported.load(Ordering::Relaxed)
    }

    pub fn mark_unsupported(&self) {
        self.unsupported.store(true, Ordering::Relaxed);
    }

    /// Claim the newly detected flag once, even when several runs share this memo.
    pub fn take_unrecorded(&self) -> bool {
        self.unsupported() && !self.recorded.swap(true, Ordering::Relaxed)
    }
}

#[derive(Default)]
pub struct ToolStreamMemos(Mutex<HashMap<i64, (ProviderKind, String, i64, ToolStreamMemo)>>);

impl ToolStreamMemos {
    pub fn get_or_insert(&self, connection: &Connection) -> ToolStreamMemo {
        let mut memos = self.0.lock().unwrap();
        let entry = memos.entry(connection.id).or_insert_with(|| {
            (
                connection.kind,
                connection.base_url.clone(),
                connection.tool_stream_revision,
                ToolStreamMemo::new(connection.tool_stream_unsupported),
            )
        });
        if entry.2 > connection.tool_stream_revision {
            return ToolStreamMemo::new(connection.tool_stream_unsupported);
        }
        if entry.0 != connection.kind
            || entry.1 != connection.base_url
            || entry.2 != connection.tool_stream_revision
        {
            *entry = (
                connection.kind,
                connection.base_url.clone(),
                connection.tool_stream_revision,
                ToolStreamMemo::new(connection.tool_stream_unsupported),
            );
        } else if connection.tool_stream_unsupported {
            entry.3.mark_unsupported();
            entry.3.recorded.store(true, Ordering::Relaxed);
        }
        entry.3.clone()
    }
}

/// A chunk of a streamed chat completion.
pub enum StreamChunk {
    /// A content delta to append to the reply.
    Delta(String),
    Reasoning(String),
    Stop(openwebide_core::StopReason),
    /// The turn's usage, yielded exactly once, just before a successful end.
    Usage(TurnTelemetry),
}

/// `Some(Instant::now())` normally; `None` on `wasm32-unknown-unknown`, where
/// `Instant::now()` panics. Providers use this instead of calling
/// `Instant::now()` directly, so the same code compiles for the browser and
/// the timing there degrades to "unknown" (duration 0) rather than crashing.
#[allow(clippy::unnecessary_wraps)] // Browser targets have no Instant, so both targets share an optional clock.
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
                (Some(start), Some(end)) => round_ns_to_ms(
                    u64::try_from(end.saturating_duration_since(start).as_nanos())
                        .unwrap_or(u64::MAX),
                ),
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
    fn tool_stream_memo(&self) -> Option<ToolStreamMemo> {
        None
    }

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
    fn chat_tools_stream(
        &self,
        req: &ChatRequest,
    ) -> Pin<Box<dyn Stream<Item = Result<ToolStreamChunk, ProviderError>> + Send + 'static>>;
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

/// The shared rule for tool-call responses: a `tool_calls` array is only
/// meaningful when it's non-empty. `{"tool_calls": []}` means the model
/// answered with a (possibly empty) text reply, not zero tool calls to run.
pub(crate) fn tool_call_values(message: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    match message.get("tool_calls").and_then(|v| v.as_array()) {
        Some(calls) if !calls.is_empty() => Some(calls),
        _ => None,
    }
}

/// Apply validated per-model settings to either provider's completion payload.
pub(crate) fn apply_model_settings(
    body: &mut serde_json::Value,
    request: &ChatRequest,
    kind: ProviderKind,
) {
    let settings = &request.model_settings;
    if kind == ProviderKind::Ollama {
        if !settings.sampling.is_empty()
            || settings.context_limit.is_some()
            || settings.max_output_tokens.is_some()
        {
            if !body["options"].is_object() {
                body["options"] = json!({});
            }
            for (key, value) in &settings.sampling {
                body["options"][key] = value.clone();
            }
            if let Some(limit) = settings.context_limit {
                body["options"]["num_ctx"] = json!(limit);
            }
            if let Some(limit) = settings.max_output_tokens {
                body["options"]["num_predict"] = json!(limit);
            }
        }
        if let Some(thinking) = settings.thinking {
            body["think"] = json!(thinking);
        }
    } else {
        for (key, value) in &settings.sampling {
            body[key] = value.clone();
        }
        if let Some(limit) = settings.max_output_tokens {
            body["max_tokens"] = json!(limit);
        }
        if let Some(thinking) = settings.thinking {
            body["chat_template_kwargs"] = json!({"enable_thinking": thinking});
        }
    }
    if settings.tools == Some(false) {
        body.as_object_mut().unwrap().remove("tools");
    }
}

/// Request override takes precedence over the saved connection model in every provider call.
fn request_model<'a>(
    requested: Option<&'a str>,
    configured: Option<&'a str>,
) -> Result<&'a str, ProviderError> {
    requested.or(configured).ok_or(ProviderError::NoModel)
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use futures::{StreamExt, stream};

    use super::*;

    #[test]
    fn model_options_map_to_both_provider_protocols() {
        let request = ChatRequest {
            connection_id: 1,
            model: Some("main".into()),
            system_prompt: None,
            messages: vec![],
            tools: vec![],
            model_settings: openwebide_core::ModelSettings {
                context_limit: Some(8192),
                max_output_tokens: Some(512),
                sampling: std::collections::BTreeMap::from([("temperature".into(), json!(0.3))]),
                thinking: Some(false),
                tools: Some(false),
                ..Default::default()
            },
        };
        let mut ollama = json!({"tools": [], "options": {"num_ctx": 4096}});
        apply_model_settings(&mut ollama, &request, ProviderKind::Ollama);
        assert_eq!(
            ollama,
            json!({"options": {"num_ctx": 8192, "num_predict": 512, "temperature": 0.3}, "think": false})
        );
        let mut compatible = json!({"tools": []});
        apply_model_settings(&mut compatible, &request, ProviderKind::LlamaCpp);
        assert_eq!(
            compatible,
            json!({"temperature": 0.3, "max_tokens": 512, "chat_template_kwargs": {"enable_thinking": false}})
        );
    }

    #[test]
    fn connection_reset_replaces_memo_even_when_url_and_kind_return() {
        let memos = ToolStreamMemos::default();
        let mut connection = Connection {
            id: 7,
            name: "server".into(),
            kind: ProviderKind::LlamaCpp,
            base_url: "http://server-a".into(),
            model: None,
            enabled: true,
            context_limit: None,
            tool_stream_unsupported: true,
            tool_stream_revision: 0,
        };
        let original = connection.clone();
        let old_memo = memos.get_or_insert(&connection);
        assert!(old_memo.unsupported());
        connection.tool_stream_unsupported = false;
        connection.tool_stream_revision = 2;
        let fresh = memos.get_or_insert(&connection);
        let concurrent = memos.get_or_insert(&connection);
        assert!(!fresh.unsupported());
        assert!(!concurrent.unsupported());
        let stale = memos.get_or_insert(&original);
        assert!(stale.unsupported());
        assert!(!memos.get_or_insert(&connection).unsupported());
        old_memo.mark_unsupported();
        assert!(!fresh.unsupported());
        fresh.mark_unsupported();
        assert!(concurrent.unsupported());
        assert!(fresh.take_unrecorded());
        assert!(!concurrent.take_unrecorded());
        connection.tool_stream_revision = 4;
        let after_kind_reset = memos.get_or_insert(&connection);
        assert!(!after_kind_reset.unsupported());
        assert!(concurrent.unsupported());
    }

    fn byte_stream(chunks: Vec<&[u8]>) -> impl Stream<Item = Result<Bytes, ProviderError>> + Unpin {
        stream::iter(chunks.into_iter().map(|c| Ok(Bytes::from(c.to_vec()))))
    }

    #[test]
    fn line_split_inside_multi_byte_char() {
        // "héllo\n" with the 'é' (0xC3 0xA9) split across two chunks.
        let chunks: Vec<&[u8]> = vec![b"h\xc3", b"\xa9llo\n"];
        let mut lines = LineStream::new(byte_stream(chunks));
        assert_eq!(block_on(lines.next()).unwrap().unwrap(), "héllo");
        assert!(block_on(lines.next()).is_none());
    }

    #[test]
    fn crlf_is_stripped() {
        let chunks: Vec<&[u8]> = vec![b"hello\r\n"];
        let mut lines = LineStream::new(byte_stream(chunks));
        assert_eq!(block_on(lines.next()).unwrap().unwrap(), "hello");
        assert!(block_on(lines.next()).is_none());
    }

    #[test]
    fn invalid_utf8_errors_then_ends() {
        let chunks: Vec<&[u8]> = vec![b"\xff\xfe\n", b"more\n"];
        let mut lines = LineStream::new(byte_stream(chunks));
        assert!(matches!(
            block_on(lines.next()),
            Some(Err(ProviderError::Parse(_)))
        ));
        assert!(block_on(lines.next()).is_none());
    }

    #[test]
    fn line_longer_than_max_errors_then_ends() {
        let chunks: Vec<&[u8]> = vec![b"0123456789", b"more\n"];
        let mut lines = LineStream::with_max_line(byte_stream(chunks), 8);
        assert!(matches!(
            block_on(lines.next()),
            Some(Err(ProviderError::Parse(_)))
        ));
        assert!(block_on(lines.next()).is_none());
    }

    #[test]
    fn final_line_without_trailing_newline() {
        let chunks: Vec<&[u8]> = vec![b"first\n", b"last"];
        let mut lines = LineStream::new(byte_stream(chunks));
        assert_eq!(block_on(lines.next()).unwrap().unwrap(), "first");
        assert_eq!(block_on(lines.next()).unwrap().unwrap(), "last");
        assert!(block_on(lines.next()).is_none());
    }

    #[test]
    fn line_crossing_max_in_same_chunk_as_newline_errors() {
        let chunks: Vec<&[u8]> = vec![b"0123456789more\n"];
        let mut lines = LineStream::with_max_line(byte_stream(chunks), 8);
        assert!(matches!(
            block_on(lines.next()),
            Some(Err(ProviderError::Parse(_)))
        ));
        assert!(block_on(lines.next()).is_none());
    }

    #[test]
    fn final_line_with_lone_trailing_cr_preserves_it() {
        let chunks: Vec<&[u8]> = vec![b"result;\r"];
        let mut lines = LineStream::new(byte_stream(chunks));
        assert_eq!(block_on(lines.next()).unwrap().unwrap(), "result;\r");
        assert!(block_on(lines.next()).is_none());
    }
    #[test]
    fn completion_queue_replays_preamble_text_usage_and_errors() {
        use openwebide_core::{ChatResponse, ToolCall};
        let usage = TurnTelemetry {
            prompt_tokens: 10,
            completion_tokens: 2,
            eval_duration_ms: 4,
            estimated: false,
        };
        let response = ChatResponse::ToolCalls(vec![ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        }]);
        let provider = fake::FakeProvider::new(vec![
            Ok(ChatCompletion {
                reasoning: String::new(),
                stop_reason: openwebide_core::StopReason::Complete,
                preamble: "Checking".into(),
                response: response.clone(),
                usage: Some(usage),
            }),
            Ok(ChatCompletion {
                reasoning: String::new(),
                stop_reason: openwebide_core::StopReason::Complete,
                preamble: String::new(),
                response: ChatResponse::Text("done".into()),
                usage: None,
            }),
            Err(ProviderError::Incomplete),
            Ok(ChatCompletion {
                reasoning: String::new(),
                stop_reason: openwebide_core::StopReason::Complete,
                preamble: String::new(),
                response: ChatResponse::Text(String::new()),
                usage: None,
            }),
        ]);
        let request = ChatRequest {
            model_settings: Default::default(),
            connection_id: 1,
            model: None,
            system_prompt: None,
            messages: vec![],
            tools: vec![],
        };
        assert_eq!(
            block_on(provider.chat_tools_stream(&request).collect::<Vec<_>>())
                .into_iter()
                .map(Result::unwrap)
                .collect::<Vec<_>>(),
            vec![
                ToolStreamChunk::Delta("Checking".into()),
                ToolStreamChunk::Stop(openwebide_core::StopReason::Complete),
                ToolStreamChunk::Usage(usage),
                ToolStreamChunk::Response(response),
            ]
        );
        assert_eq!(
            block_on(provider.chat_tools_stream(&request).collect::<Vec<_>>())
                .into_iter()
                .map(Result::unwrap)
                .collect::<Vec<_>>(),
            vec![
                ToolStreamChunk::Delta("done".into()),
                ToolStreamChunk::Stop(openwebide_core::StopReason::Complete),
                ToolStreamChunk::Response(ChatResponse::Text("done".into())),
            ]
        );
        let items = block_on(provider.chat_tools_stream(&request).collect::<Vec<_>>());
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Err(ProviderError::Incomplete)));
        assert_eq!(
            block_on(provider.chat_tools_stream(&request).collect::<Vec<_>>())
                .into_iter()
                .map(Result::unwrap)
                .collect::<Vec<_>>(),
            vec![
                ToolStreamChunk::Stop(openwebide_core::StopReason::Complete),
                ToolStreamChunk::Response(ChatResponse::Text(String::new()))
            ]
        );
    }
}
