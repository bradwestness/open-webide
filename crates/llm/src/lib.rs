//! LLM provider abstraction for Open WebIDE.
//!
//! Providers are constructed from a saved
//! [`Connection`](openwebide_core::Connection) via
//! [`registry::Provider::for_connection`].

pub mod error;
pub mod llamacpp;
pub mod ollama;
pub mod registry;

pub(crate) mod sse;

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
///
/// Step 39's `chat_tools_stream` uses the same parsers and terminal rule.
pub(crate) enum StreamLine {
    /// A content delta to emit.
    Delta(String),
    /// The model finished; may carry a last delta. More metadata lines may
    /// follow; EOF is now acceptable.
    Finished(Option<String>),
    /// The stream finished successfully.
    Done,
    /// A keep-alive or metadata line to ignore.
    Skip,
}

/// The default cap on a single stream line's byte length. Ollama puts a
/// whole tool call, including a full `write_file` body, on one NDJSON
/// line, so this is generous.
const MAX_LINE_BYTES: usize = 16 << 20;

/// Splits a byte-chunk stream into newline-delimited lines.
///
/// Handles chunks that split a line (or a multi-byte UTF-8 character)
/// mid-way, `\r\n` line endings, and a final line without a trailing
/// newline. The buffer is bounded by `max_line`: a line longer than that
/// yields an error and fuses the stream.
pub(crate) struct LineStream<S> {
    inner: S,
    buffer: Vec<u8>,
    scan_from: usize,
    max_line: usize,
    done: bool,
}

impl<S> LineStream<S> {
    pub(crate) fn new(inner: S) -> Self {
        Self::with_max_line(inner, MAX_LINE_BYTES)
    }

    pub(crate) fn with_max_line(inner: S, max_line: usize) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            scan_from: 0,
            max_line,
            done: false,
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
        if this.done {
            return Poll::Ready(None);
        }
        loop {
            if let Some(pos) = this.buffer[this.scan_from..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|p| p + this.scan_from)
            {
                if pos + 1 > this.max_line {
                    this.done = true;
                    let max_line = this.max_line;
                    return Poll::Ready(Some(Err(ProviderError::Parse(format!(
                        "stream line exceeds {max_line} bytes"
                    )))));
                }
                let mut end = pos;
                if end > 0 && this.buffer[end - 1] == b'\r' {
                    end -= 1;
                }
                let line_bytes: Vec<u8> = this.buffer[..end].to_vec();
                this.buffer.drain(..=pos);
                this.scan_from = 0;
                if line_bytes.is_empty() {
                    continue;
                }
                return match String::from_utf8(line_bytes) {
                    Ok(line) => Poll::Ready(Some(Ok(line))),
                    Err(_) => {
                        this.done = true;
                        Poll::Ready(Some(Err(ProviderError::Parse(
                            "stream line is not valid UTF-8".into(),
                        ))))
                    }
                };
            }
            this.scan_from = this.buffer.len();
            if this.buffer.len() > this.max_line {
                this.done = true;
                let max_line = this.max_line;
                return Poll::Ready(Some(Err(ProviderError::Parse(format!(
                    "stream line exceeds {max_line} bytes"
                )))));
            }
            match Stream::poll_next(std::pin::Pin::new(&mut this.inner), cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    this.buffer.extend_from_slice(&chunk);
                }
                Poll::Ready(Some(Err(e))) => {
                    this.done = true;
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(None) => {
                    this.done = true;
                    let line_bytes = std::mem::take(&mut this.buffer);
                    if line_bytes.is_empty() {
                        return Poll::Ready(None);
                    }
                    return match String::from_utf8(line_bytes) {
                        Ok(line) => Poll::Ready(Some(Ok(line))),
                        Err(_) => Poll::Ready(Some(Err(ProviderError::Parse(
                            "stream line is not valid UTF-8".into(),
                        )))),
                    };
                }
                Poll::Pending => return Poll::Pending,
            }
        }
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

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use futures::{StreamExt, stream};

    use super::*;

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
}
