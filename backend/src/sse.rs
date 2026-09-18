//! Server-Sent Events for streaming chat responses.
//!
//! One SSE stream per sent message: the user message (already persisted),
//! then either content deltas (plain chat) or agent tool steps (agentic
//! coding), and finally the persisted assistant message. Frame format:
//!
//! ```text
//! event: message | delta | tool_call | tool_result | done | error
//! data: {json}
//!
//! ```

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::{Stream, StreamExt, stream};
use http_body::{Frame, SizeHint};
use openwebide_core::{ChatMessage, ChatRequest, FileDiff, Role};
use openwebide_llm::{LlmProvider, registry::Provider};
use openwebide_storage::{Store, spin_db::SpinDb};
use serde_json::json;

use crate::http_client::SpinHttpClient;
use crate::state::now;

/// One event on a chat SSE stream.
pub enum SseEvent {
    /// The user message that was persisted before the provider call.
    Message(ChatMessage),
    /// A content delta from the provider.
    Delta(String),
    /// The agent requested a tool call.
    ToolCall {
        id: String,
        name: String,
        summary: String,
    },
    /// A tool call finished.
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        summary: String,
        diff: Option<FileDiff>,
    },
    /// The persisted assistant message; the stream ends after this.
    Done(ChatMessage),
    /// A failure; the stream ends after this.
    Error(String),
}

/// Encode one event as an SSE frame.
fn frame(event: &SseEvent) -> Bytes {
    let (name, data) = match event {
        SseEvent::Message(message) => ("message", serde_json::to_string(message).unwrap()),
        SseEvent::Delta(delta) => ("delta", json!({ "content": delta }).to_string()),
        SseEvent::ToolCall { id, name, summary } => (
            "tool_call",
            json!({ "id": id, "name": name, "summary": summary }).to_string(),
        ),
        SseEvent::ToolResult {
            id,
            name,
            ok,
            summary,
            diff,
        } => (
            "tool_result",
            json!({ "id": id, "name": name, "ok": ok, "summary": summary, "diff": diff })
                .to_string(),
        ),
        SseEvent::Done(message) => ("done", serde_json::to_string(message).unwrap()),
        SseEvent::Error(error) => ("error", json!({ "error": error }).to_string()),
    };
    Bytes::from(format!("event: {name}\ndata: {data}\n\n"))
}

/// An `http_body::Body` that writes SSE frames as they are produced.
pub struct SseBody {
    stream: Pin<Box<dyn Stream<Item = SseEvent> + Send>>,
}

impl SseBody {
    pub fn new(stream: Pin<Box<dyn Stream<Item = SseEvent> + Send + 'static>>) -> Self {
        Self { stream }
    }
}

impl http_body::Body for SseBody {
    type Data = Bytes;
    type Error = anyhow::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        // Provider and persistence failures already surface as
        // `SseEvent::Error` inside the stream, so every item is a frame.
        match Stream::poll_next(this.stream.as_mut(), cx) {
            Poll::Ready(Some(event)) => Poll::Ready(Some(Ok(Frame::data(frame(&event))))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        false
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

/// Shared state between the delta mapping and the tail that persists the
/// assistant message.
struct StreamState {
    buffer: String,
    failed: bool,
}

/// Build the SSE event stream for one sent message: the user message, the
/// provider's deltas, and (on success) the persisted assistant message.
///
/// The store is moved in: the assistant message is persisted from inside the
/// stream, so the response body outlives the request handler.
pub fn message_stream(
    store: Store<SpinDb>,
    session_id: i64,
    user_message: ChatMessage,
    request: ChatRequest,
    provider: Provider<SpinHttpClient>,
) -> Pin<Box<dyn Stream<Item = SseEvent> + Send + 'static>> {
    let state = Arc::new(Mutex::new(StreamState {
        buffer: String::new(),
        failed: false,
    }));
    let deltas = {
        let state = state.clone();
        provider
            .chat_stream(&request)
            .map(move |result| match result {
                Ok(delta) => {
                    state.lock().unwrap().buffer.push_str(&delta);
                    SseEvent::Delta(delta)
                }
                Err(error) => {
                    state.lock().unwrap().failed = true;
                    SseEvent::Error(error.to_string())
                }
            })
    };
    // Persist the accumulated reply. On failure the partial reply is not
    // saved, so history never contains a truncated assistant message. The
    // `Option` state makes this a one-shot: after emitting, `maybe?` ends
    // the stream.
    let tail = stream::unfold(Some((store, session_id, state)), |maybe| async move {
        let (store, session_id, state) = maybe?;
        let (failed, content) = {
            let st = state.lock().unwrap();
            (st.failed, st.buffer.clone())
        };
        if failed {
            return None;
        }
        match store
            .insert_message(session_id, Role::Assistant, &content, now())
            .await
        {
            Ok(message) => Some((SseEvent::Done(message), None)),
            Err(error) => Some((
                SseEvent::Error(format!("failed to save reply: {error}")),
                None,
            )),
        }
    });
    Box::pin(
        stream::iter([SseEvent::Message(user_message)])
            .chain(deltas)
            .chain(tail),
    )
}
