//! Server-Sent Events for streaming chat responses.
//!
//! One SSE stream per sent message: the user message (already persisted),
//! then either content deltas (plain chat) or agent tool steps (agentic
//! coding), and finally the persisted assistant message. Frame format:
//!
//! ```text
//! event: message | delta | tool_call | permission_request | tool_result | done | telemetry | cancelled | error
//! data: {json}
//!
//! ```

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::{Stream, StreamExt, stream};
use http_body::{Frame, SizeHint};
use openwebide_core::{ChatMessage, FileDiff, Role, TurnTelemetry};
use openwebide_llm::{ProviderError, StreamChunk};
use openwebide_storage::Store;
use serde_json::json;

use crate::state::{AppDb, now};

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
    /// A gated tool call is waiting for the user's approval; a
    /// `ToolResult` follows once the decision is in (either way).
    PermissionRequest {
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
    /// Telemetry metrics for the turn.
    Telemetry(TurnTelemetry),
    /// The user cancelled the run; the stream ends after this.
    Cancelled,
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
        SseEvent::PermissionRequest { id, name, summary } => (
            "permission_request",
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
        SseEvent::Telemetry(telem) => (
            "telemetry",
            serde_json::to_string(telem).unwrap_or_default(),
        ),
        SseEvent::Cancelled => ("cancelled", "{}".to_string()),
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

/// Plain-chat stream state: accumulates the reply, remembers the turn's
/// usage, and ends after the first terminal outcome (cancel, provider
/// error, or provider end-of-stream).
struct StreamState {
    store: Arc<Store<AppDb>>,
    session_id: i64,
    chunks: Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>>,
    buffer: String,
    usage: Option<TurnTelemetry>,
    done: bool,
}

/// Build the SSE event stream for one sent message: the user message, the
/// provider's deltas, and a terminal event — the persisted assistant
/// message on success, `Cancelled` if the user stops the run, or `Error`
/// if the provider or persistence fails. The stream always ends with its
/// terminal event, never a bare end-of-stream.
///
/// The store is shared: the cancel gate polls the database for a cancel
/// request (arriving as a separate Spin request) and the tail persists the
/// reply, so the response body outlives the request handler.
pub fn message_stream(
    store: Arc<Store<AppDb>>,
    session_id: i64,
    user_message: ChatMessage,
    chunks: Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>>,
) -> Pin<Box<dyn Stream<Item = SseEvent> + Send + 'static>> {
    let state = StreamState {
        store,
        session_id,
        chunks,
        buffer: String::new(),
        usage: None,
        done: false,
    };
    let machine = stream::unfold(state, |mut state| async move {
        if state.done {
            return None;
        }
        match state.chunks.as_mut().next().await {
            Some(Ok(StreamChunk::Delta(delta))) => {
                // Poll the cancel flag as each delta arrives; a cancel
                // requested by a separate request ends the stream with
                // `Cancelled`, and the triggering delta is not sent.
                if state
                    .store
                    .cancel_requested(state.session_id)
                    .await
                    .unwrap_or(false)
                {
                    state.done = true;
                    let _ = state.store.clear_cancel(state.session_id).await;
                    Some((SseEvent::Cancelled, state))
                } else {
                    state.buffer.push_str(&delta);
                    Some((SseEvent::Delta(delta), state))
                }
            }
            Some(Ok(StreamChunk::Usage(usage))) => {
                state.usage = Some(usage);
                Some((SseEvent::Telemetry(usage), state))
            }
            Some(Err(error)) => {
                state.done = true;
                let _ = state.store.clear_cancel(state.session_id).await;
                Some((SseEvent::Error(error.to_string()), state))
            }
            None => {
                // The run is over: drop the flag so a late or stale cancel
                // can't affect the next run.
                state.done = true;
                let _ = state.store.clear_cancel(state.session_id).await;
                // Persist the accumulated reply. On failure the partial
                // reply is not saved, so history never contains a
                // truncated assistant message.
                match state
                    .store
                    .insert_message_with_usage(
                        state.session_id,
                        Role::Assistant,
                        &state.buffer,
                        now(),
                        state.usage.as_ref(),
                    )
                    .await
                {
                    Ok(message) => Some((SseEvent::Done(message), state)),
                    Err(error) => Some((
                        SseEvent::Error(format!("failed to save reply: {error}")),
                        state,
                    )),
                }
            }
        }
    });
    Box::pin(stream::iter([SseEvent::Message(user_message)]).chain(machine))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use openwebide_core::UserRole;

    /// An in-memory store with a user and a session, for stream tests.
    async fn test_store() -> (Arc<Store<AppDb>>, i64) {
        let db = AppDb::open_in_memory().unwrap();
        let store = Arc::new(Store::new(db));
        store.migrate().await.unwrap();
        let user = store
            .insert_user("tester", "hash", UserRole::User, 1)
            .await
            .unwrap();
        let session = store
            .create_session("test", None, None, None, user.id, 1)
            .await
            .unwrap();
        (store, session.id)
    }

    /// A fake provider chunk stream.
    fn fake_chunks(
        items: Vec<Result<StreamChunk, ProviderError>>,
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
        Box::pin(stream::iter(items))
    }

    fn delta(s: &str) -> Result<StreamChunk, ProviderError> {
        Ok(StreamChunk::Delta(s.to_string()))
    }

    /// The event reduced to the fields the assertions care about.
    #[derive(Debug, PartialEq)]
    enum Kind {
        Message,
        Delta(String),
        Telemetry(TurnTelemetry),
        Done(String),
        Cancelled,
        Error(String),
    }

    fn kind(event: &SseEvent) -> Kind {
        match event {
            SseEvent::Message(_) => Kind::Message,
            SseEvent::Delta(d) => Kind::Delta(d.clone()),
            SseEvent::ToolCall { .. } => panic!("unexpected tool_call"),
            SseEvent::PermissionRequest { .. } => panic!("unexpected permission_request"),
            SseEvent::ToolResult { .. } => panic!("unexpected tool_result"),
            SseEvent::Done(m) => Kind::Done(m.content.clone()),
            SseEvent::Telemetry(t) => Kind::Telemetry(*t),
            SseEvent::Cancelled => Kind::Cancelled,
            SseEvent::Error(e) => Kind::Error(e.clone()),
        }
    }

    /// Persist a user message, run the stream over the fake chunks, and
    /// return the event sequence.
    async fn run(
        store: Arc<Store<AppDb>>,
        session_id: i64,
        items: Vec<Result<StreamChunk, ProviderError>>,
    ) -> Vec<Kind> {
        let user_message = store
            .insert_message(session_id, Role::User, "hello", now())
            .await
            .unwrap();
        let events = message_stream(store, session_id, user_message, fake_chunks(items))
            .collect::<Vec<_>>()
            .await;
        events.iter().map(kind).collect()
    }

    #[test]
    fn provider_error_yields_error_event_and_persists_nothing() {
        block_on(async {
            let (store, session_id) = test_store().await;
            let events = run(
                store.clone(),
                session_id,
                vec![
                    delta("a"),
                    Err(ProviderError::Http("401 Unauthorized".to_string())),
                ],
            )
            .await;
            assert_eq!(
                events,
                vec![
                    Kind::Message,
                    Kind::Delta("a".to_string()),
                    Kind::Error("HTTP error: 401 Unauthorized".to_string()),
                ]
            );
            // The partial reply is not saved: only the user message exists.
            let messages = store.list_messages(session_id).await.unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, Role::User);
        });
    }

    #[test]
    fn cancel_before_first_delta_yields_cancelled_and_persists_nothing() {
        block_on(async {
            let (store, session_id) = test_store().await;
            store.request_cancel(session_id).await.unwrap();
            let events = run(store.clone(), session_id, vec![delta("a"), delta("b")]).await;
            assert_eq!(events, vec![Kind::Message, Kind::Cancelled]);
            // The partial reply is not saved, and the flag is cleared so
            // the next run starts clean.
            let messages = store.list_messages(session_id).await.unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, Role::User);
            assert!(!store.cancel_requested(session_id).await.unwrap());
        });
    }

    #[test]
    fn usage_is_stored_with_the_persisted_reply() {
        block_on(async {
            let (store, session_id) = test_store().await;
            let usage = TurnTelemetry {
                prompt_tokens: 10,
                completion_tokens: 20,
                eval_duration_ms: 100,
                estimated: false,
            };
            let events = run(
                store.clone(),
                session_id,
                vec![delta("a"), Ok(StreamChunk::Usage(usage)), delta("b")],
            )
            .await;
            assert_eq!(
                events,
                vec![
                    Kind::Message,
                    Kind::Delta("a".to_string()),
                    Kind::Telemetry(usage),
                    Kind::Delta("b".to_string()),
                    Kind::Done("ab".to_string()),
                ]
            );
            let messages = store.list_messages(session_id).await.unwrap();
            assert_eq!(messages.len(), 2);
            let assistant = &messages[1];
            assert_eq!(assistant.role, Role::Assistant);
            assert_eq!(assistant.content, "ab");
            assert_eq!(assistant.usage, Some(usage));
        });
    }

    #[test]
    fn empty_successful_reply_persists_empty_content() {
        block_on(async {
            let (store, session_id) = test_store().await;
            let events = run(store.clone(), session_id, Vec::new()).await;
            assert_eq!(events, vec![Kind::Message, Kind::Done(String::new())]);
            let messages = store.list_messages(session_id).await.unwrap();
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[1].role, Role::Assistant);
            assert_eq!(messages[1].content, "");
            assert_eq!(messages[1].usage, None);
        });
    }
}
