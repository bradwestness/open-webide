//! Server-Sent Events for streaming chat responses.
//!
//! One SSE stream per sent message: the user message (already persisted),
//! then either content deltas (plain chat) or agent tool steps (agentic
//! coding), and finally the persisted assistant message. Frame format:
//!
//! ```text
//! event: message | delta | reasoning_delta | tool_call | permission_request | tool_result | interim | done | telemetry | cancelled | error
//! data: {json}
//!
//! ```

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::{Stream, StreamExt, stream};
use http_body::{Frame, SizeHint};
use openwebide_core::{ChatMessage, RunEvent};
use openwebide_llm::{ProviderError, StreamChunk};
use openwebide_storage::Store;

use crate::state::AppDb;

/// Encode one event as an SSE frame.
fn frame(event: &RunEvent) -> Bytes {
    let name = event.kind_str();
    let data = serde_json::to_string(event).unwrap();
    Bytes::from(format!("event: {name}\ndata: {data}\n\n"))
}

/// An `http_body::Body` that writes SSE frames as they are produced.
pub struct SseBody {
    stream: Pin<Box<dyn Stream<Item = RunEvent> + Send>>,
}

impl SseBody {
    pub fn new(stream: Pin<Box<dyn Stream<Item = RunEvent> + Send + 'static>>) -> Self {
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
        // `RunEvent::Error` inside the stream, so every item is a frame.
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

#[derive(Default)]
struct CancelPoll {
    last: Option<std::time::Instant>,
}

impl CancelPoll {
    fn due(&mut self, now: std::time::Instant) -> bool {
        if self.last.is_some_and(|last| {
            now.saturating_duration_since(last) < std::time::Duration::from_millis(250)
        }) {
            return false;
        }
        self.last = Some(now);
        true
    }
}

struct ChatCancel {
    store: Arc<Store<AppDb>>,
    session_id: i64,
    started_ms: i64,
    poll: std::sync::Mutex<CancelPoll>,
}
impl openwebide_agent::CancelCheck for ChatCancel {
    async fn check(&self) -> bool {
        if !self.poll.lock().unwrap().due(std::time::Instant::now()) {
            return false;
        }
        match self
            .store
            .cancel_requested_since(self.session_id, self.started_ms)
            .await
        {
            Ok(cancelled) => cancelled,
            Err(error) => {
                eprintln!(
                    "session {}: cancel_requested_since: {error}",
                    self.session_id
                );
                false
            }
        }
    }
    fn cancelled(&self) -> impl std::future::Future<Output = ()> + Send {
        // Spin's database cancel flag is polled at stream boundaries; it has no push notification.
        std::future::pending()
    }
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
struct ChatPersistence {
    user: openwebide_core::UserId,
    store: Arc<Store<AppDb>>,
    session: i64,
}
impl openwebide_agent::session::ChatPersistence for ChatPersistence {
    async fn notify(&self, event: &openwebide_core::push::RunNotification) -> Result<(), String> {
        self.store.queue_run_notification(self.user, self.session, event, crate::state::now()).await.map_err(|error| error.to_string())
    }
    async fn save_reply(
        &self,
        content: &str,
        usage: Option<&openwebide_core::TurnTelemetry>,
    ) -> Result<ChatMessage, String> {
        self.store
            .insert_message_with_usage(
                self.session,
                openwebide_core::Role::Assistant,
                content,
                crate::state::now(),
                usage,
            )
            .await
            .map_err(|error| error.to_string())
    }
}

pub fn message_stream(
    store: Arc<Store<AppDb>>,
    user: openwebide_core::UserId,
    session_id: i64,
    user_message: ChatMessage,
    chunks: Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>>,
    started_ms: i64,
    request: &openwebide_core::ChatRequest,
) -> Pin<Box<dyn Stream<Item = RunEvent> + Send + 'static>> {
    let cancel = ChatCancel {
        store: store.clone(),
        session_id,
        started_ms,
        poll: Default::default(),
    };
    let machine = openwebide_agent::session::chat_events(
        ChatPersistence {
            user,
            store,
            session: session_id,
        },
        chunks,
        cancel,
        request,
    );
    Box::pin(
        stream::iter([RunEvent::Message {
            message: user_message,
        }])
        .chain(machine),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::now;
    use futures::executor::block_on;
    use openwebide_core::UserRole;
    use openwebide_core::{REPLY_TRUNCATED_MARKER, Role, TurnTelemetry};

    #[test]
    fn cancel_poll_is_throttled() {
        let mut poll = CancelPoll::default();
        let start = std::time::Instant::now();
        assert!(poll.due(start));
        assert!(!poll.due(start));
        assert!(!poll.due(start + std::time::Duration::from_millis(249)));
        assert!(poll.due(start + std::time::Duration::from_millis(250)));
        assert!(!poll.due(start + std::time::Duration::from_millis(499)));
        assert!(poll.due(start + std::time::Duration::from_millis(500)));
    }

    #[test]
    fn frames_encode_tagged_run_events() {
        let message = ChatMessage {
            id: 2,
            session_id: 1,
            role: Role::Assistant,
            content: "reply".into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        };
        for (event, expected) in [
            (
                RunEvent::Delta {
                    content: "hi".into(),
                },
                "event: delta\ndata: {\"kind\":\"delta\",\"content\":\"hi\"}\n\n",
            ),
            (
                RunEvent::Done { message },
                "event: done\ndata: {\"kind\":\"done\",\"message\":{\"id\":2,\"session_id\":1,\"role\":\"assistant\",\"content\":\"reply\",\"created_at\":0}}\n\n",
            ),
            (
                RunEvent::Error {
                    message: "failed".into(),
                },
                "event: error\ndata: {\"kind\":\"error\",\"message\":\"failed\"}\n\n",
            ),
        ] {
            assert_eq!(frame(&event).as_ref(), expected.as_bytes());
        }
    }

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

    fn delta(s: &str) -> StreamChunk {
        StreamChunk::Delta(s.to_string())
    }

    /// The event reduced to the fields the assertions care about.
    #[derive(Debug, PartialEq)]
    enum Kind {
        Message,
        Delta(String),
        Reasoning(String),
        Telemetry(TurnTelemetry),
        Done(String),
        Cancelled,
        Error(String),
    }

    fn kind(event: &RunEvent) -> Kind {
        match event {
            RunEvent::Message { .. } => Kind::Message,
            RunEvent::Delta { content: d } => Kind::Delta(d.clone()),
            RunEvent::ReasoningDelta { content } => Kind::Reasoning(content.clone()),
            RunEvent::Task { .. } => panic!("unexpected child task in plain chat"),
            RunEvent::Interim { .. } => panic!("unexpected interim"),
            RunEvent::ToolCall { .. } => panic!("unexpected tool_call"),
            RunEvent::PermissionRequest { .. } => panic!("unexpected permission_request"),
            RunEvent::ToolResult { .. } => panic!("unexpected tool_result"),
            RunEvent::ToolTiming { .. } => panic!("unexpected tool_timing"),
            RunEvent::Done { message: m } => Kind::Done(m.content.clone()),
            RunEvent::Telemetry { usage: t } => Kind::Telemetry(*t),
            RunEvent::Cancelled => Kind::Cancelled,
            RunEvent::Error { message: e } => Kind::Error(e.clone()),
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
        let events = message_stream(
            store,
            openwebide_core::UserId::new(1),
            session_id,
            user_message,
            fake_chunks(items),
            1000,
            &openwebide_core::ChatRequest {
                connection_id: 1,
                model: None,
                system_prompt: None,
                model_settings: Default::default(),
                messages: vec![],
                tools: vec![],
            },
        )
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
                    Ok(delta("a")),
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
    fn incomplete_stream_with_partial_reply_persists_truncated_marker() {
        block_on(async {
            let (store, session_id) = test_store().await;
            let events = run(
                store.clone(),
                session_id,
                vec![Ok(delta("ab")), Err(ProviderError::Incomplete)],
            )
            .await;
            assert_eq!(
                events,
                vec![
                    Kind::Message,
                    Kind::Delta("ab".to_string()),
                    Kind::Done("ab\n\n[reply truncated]".to_string()),
                ]
            );
            // The partial reply is saved with the truncation marker and no
            // usage.
            let messages = store.list_messages(session_id).await.unwrap();
            assert_eq!(messages.len(), 2);
            let assistant = &messages[1];
            assert_eq!(assistant.role, Role::Assistant);
            assert_eq!(assistant.content, "ab\n\n[reply truncated]");
            assert_eq!(assistant.usage, None);
        });
    }

    #[test]
    fn incomplete_reasoning_only_reply_is_persisted() {
        block_on(async {
            let (store, session_id) = test_store().await;
            let reasoning = "analysis with </think> literal";
            let events = run(
                store.clone(),
                session_id,
                vec![
                    Ok(StreamChunk::Reasoning(reasoning.into())),
                    Err(ProviderError::Incomplete),
                ],
            )
            .await;
            let content = openwebide_core::with_reasoning(reasoning, REPLY_TRUNCATED_MARKER);
            assert_eq!(
                events,
                vec![
                    Kind::Message,
                    Kind::Reasoning(reasoning.into()),
                    Kind::Done(content.clone())
                ]
            );
            let messages = store.list_messages(session_id).await.unwrap();
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[1].content, content);
            assert_eq!(messages[1].usage, None);
        });
    }

    #[test]
    fn incomplete_stream_with_empty_buffer_yields_error_and_persists_nothing() {
        block_on(async {
            let (store, session_id) = test_store().await;
            let events = run(
                store.clone(),
                session_id,
                vec![Err(ProviderError::Incomplete)],
            )
            .await;
            assert_eq!(
                events,
                vec![
                    Kind::Message,
                    Kind::Error(
                        "stream ended before the model finished; the reply may be truncated"
                            .to_string()
                    ),
                ]
            );
            // Nothing is saved: only the user message exists.
            let messages = store.list_messages(session_id).await.unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, Role::User);
        });
    }

    #[test]
    fn cancel_before_first_delta_yields_cancelled_and_persists_nothing() {
        block_on(async {
            let (store, session_id) = test_store().await;
            store.request_cancel(session_id, 1500).await.unwrap();
            let events = run(
                store.clone(),
                session_id,
                vec![Ok(delta("a")), Ok(delta("b"))],
            )
            .await;
            assert_eq!(events, vec![Kind::Message, Kind::Cancelled]);
            // The partial reply is not saved; later runs ignore the old cancel.
            let messages = store.list_messages(session_id).await.unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, Role::User);
            assert!(
                store
                    .cancel_requested_since(session_id, 1000)
                    .await
                    .unwrap()
            );
            assert!(
                !store
                    .cancel_requested_since(session_id, 1600)
                    .await
                    .unwrap()
            );
        });
    }

    #[test]
    fn usage_is_stored_with_the_persisted_reply() {
        block_on(async {
            let (store, session_id) = test_store().await;
            let usage = TurnTelemetry {
                context: Some(openwebide_core::ContextBreakdown {
                    history: 10,
                    ..Default::default()
                }),
                prompt_tokens: 10,
                completion_tokens: 20,
                eval_duration_ms: 100,
                estimated: false,
            };
            let events = run(
                store.clone(),
                session_id,
                vec![
                    Ok(delta("a")),
                    Ok(StreamChunk::Usage(usage)),
                    Ok(delta("b")),
                ],
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
    #[test]
    fn reasoning_and_cutoff_are_persisted_in_plain_chat() {
        block_on(async {
            let (store, session_id) = test_store().await;
            let events = run(
                store.clone(),
                session_id,
                vec![
                    Ok(StreamChunk::Reasoning("r".into())),
                    Ok(delta("answer")),
                    Ok(StreamChunk::Stop(openwebide_core::StopReason::Length)),
                ],
            )
            .await;
            let content = format!(
                "<think>r</think>answer{}",
                openwebide_core::REPLY_CUT_OFF_MARKER
            );
            assert_eq!(
                events,
                vec![
                    Kind::Message,
                    Kind::Reasoning("r".into()),
                    Kind::Delta("answer".into()),
                    Kind::Done(content.clone())
                ]
            );
            assert_eq!(
                store.list_messages(session_id).await.unwrap()[1].content,
                content
            );
        });
    }
}
