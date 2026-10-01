//! Agentic coding for remote-mode projects: the backend's `ToolExecutor`
//! (workspace-confined file tools) and the SSE stream that wraps the agent
//! loop from the `openwebide-agent` crate.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::files::HostFsVfs;
use crate::http_client::SpinHttpClient;
use crate::state::now;
use futures::{Stream, StreamExt, stream};
use openwebide_agent::{AgentConfig, AgentEvent, CancelCheck, PermissionGate};
use openwebide_agent::{VfsToolExecutor, vfs_tools};
use openwebide_core::{
    ChatMessage, ChatRequest, Role, RunEvent, ToolCall, ToolDefinition, TurnTelemetry,
};
use openwebide_llm::registry::Provider;
use openwebide_storage::Store;

use crate::state::AppDb;

/// The workspace tools offered to the model.
pub fn workspace_tools() -> Vec<ToolDefinition> {
    vfs_tools()
}

/// A per-session cancel flag backed by SQLite. The cancel POST arrives as a
/// separate Spin request (stateless, possibly another component instance),
/// so the flag lives in the database; the in-flight stream polls it at step
/// boundaries.
pub struct CancelFlag {
    store: Arc<Store<AppDb>>,
    session_id: i64,
}

impl CancelFlag {
    pub fn new(store: Arc<Store<AppDb>>, session_id: i64) -> Self {
        Self { store, session_id }
    }
}

impl CancelCheck for CancelFlag {
    fn check(&self) -> impl Future<Output = bool> + Send {
        let store = self.store.clone();
        let session_id = self.session_id;
        async move { store.cancel_requested(session_id).await.unwrap_or(false) }
    }
}

/// How long the gate waits for the user's decision before denying.
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(300);
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// A per-session permission gate backed by SQLite. The user's decision arrives
/// as a separate Spin request (stateless, possibly another component
/// instance), so the in-flight stream polls the database until the decision
/// is recorded, the run is cancelled, or the wait times out.
pub struct PermissionPoller {
    store: Arc<Store<AppDb>>,
    session_id: i64,
}

impl PermissionPoller {
    pub fn new(store: Arc<Store<AppDb>>, session_id: i64) -> Self {
        Self { store, session_id }
    }
}

impl PermissionGate for PermissionPoller {
    fn approve(&self, call: &ToolCall) -> impl Future<Output = bool> + Send {
        let store = self.store.clone();
        let session_id = self.session_id;
        let tool_call_id = call.id.clone();
        async move {
            let started = Instant::now();
            loop {
                if let Some(decision) = store
                    .take_tool_permission(session_id, &tool_call_id)
                    .await
                    .unwrap_or(None)
                {
                    return decision;
                }
                // A cancel landing while waiting also denies the call; the
                // loop re-checks the cancel flag and reports `Cancelled`.
                if store.cancel_requested(session_id).await.unwrap_or(false) {
                    return false;
                }
                if started.elapsed() >= PERMISSION_TIMEOUT {
                    return false;
                }
                std::thread::sleep(PERMISSION_POLL_INTERVAL);
            }
        }
    }
}

/// Build the SSE event stream for one agentic message: the user message, the
/// agent's tool steps, and (on success) the persisted assistant message.
///
/// The store is shared: the agent loop polls the cancel flag and permission
/// decisions from inside the stream and the tail persists the reply, so the
/// response body outlives the request handler.
#[allow(clippy::too_many_arguments)]
pub fn agent_stream(
    store: Arc<Store<AppDb>>,
    session_id: i64,
    user_message: ChatMessage,
    request: ChatRequest,
    provider: Provider<SpinHttpClient>,
    base: String,
    config: AgentConfig,
    cancel: CancelFlag,
    gate: PermissionPoller,
) -> Pin<Box<dyn Stream<Item = RunEvent> + Send + 'static>> {
    let executor = VfsToolExecutor::with_web_and_bridge(
        HostFsVfs::new(base.clone()),
        crate::web::SpinWebClient,
        crate::bridge_client::SpinBridgeClient::for_project(store.clone(), base),
    );
    let anchor_id = user_message.id;
    let events =
        openwebide_agent::run(provider, executor, request, config, cancel, gate, anchor_id);
    let tail = map_agent_events(store, session_id, anchor_id, events);
    Box::pin(
        stream::iter([RunEvent::Message {
            message: user_message,
        }])
        .chain(tail),
    )
}

fn map_agent_events(
    store: Arc<Store<AppDb>>,
    session_id: i64,
    anchor_id: i64,
    events: impl Stream<Item = AgentEvent> + Send + 'static,
) -> impl Stream<Item = RunEvent> + Send {
    let events = Box::pin(events);
    let display_anchor = anchor_id;
    let last_usage: Option<TurnTelemetry> = None;
    stream::unfold(
        (store, session_id, display_anchor, events, last_usage),
        |state| async move {
            let (store, session_id, mut display_anchor, mut events, mut last_usage) = state;
            let Some(event) = events.next().await else {
                // The run finished (completed, failed, or cancelled): drop the
                // flag and any recorded decisions so a late or stale cancel or
                // permission can't affect the next run.
                let _ = store.clear_cancel(session_id).await;
                let _ = store.clear_tool_permissions(session_id).await;
                return None;
            };
            let sse = match event {
                AgentEvent::ToolCall { id, name, summary } => {
                    last_usage = None;
                    let _ = store
                        .upsert_tool_step(session_id, display_anchor, &id, &name, &summary, now())
                        .await;
                    RunEvent::ToolCall { id, name, summary }
                }
                AgentEvent::PermissionRequest { id, name, summary } => {
                    last_usage = None;
                    let _ = store
                        .upsert_tool_step(session_id, display_anchor, &id, &name, &summary, now())
                        .await;
                    RunEvent::PermissionRequest { id, name, summary }
                }
                AgentEvent::ToolResult {
                    id,
                    name,
                    ok,
                    summary,
                    diff,
                } => {
                    let _ = store
                        .complete_tool_step(session_id, &id, ok, &summary, diff.as_ref())
                        .await;
                    RunEvent::ToolResult {
                        id,
                        name,
                        ok,
                        summary,
                        diff,
                    }
                }
                AgentEvent::TextDelta(delta) => RunEvent::Delta { content: delta },
                AgentEvent::TurnCalls { text, calls } => {
                    let usage = last_usage.take();
                    let message = store
                        .insert_interim_message(
                            session_id,
                            Role::Assistant,
                            &text,
                            now(),
                            usage.as_ref(),
                            Some(&calls),
                        )
                        .await;
                    let message = match message {
                        Ok(message) => {
                            display_anchor = message.id;
                            message
                        }
                        Err(_) => ChatMessage {
                            id: 0,
                            session_id,
                            role: Role::Assistant,
                            content: text,
                            created_at: now(),
                            tool_calls: Some(calls),
                            tool_call_id: None,
                            usage,
                        },
                    };
                    RunEvent::Interim { message }
                }
                AgentEvent::Telemetry(usage) => {
                    last_usage = Some(usage);
                    RunEvent::Telemetry { usage }
                }
                AgentEvent::FinalText(text) => match store
                    .insert_message_with_usage(
                        session_id,
                        Role::Assistant,
                        &text,
                        now(),
                        last_usage.take().as_ref(),
                    )
                    .await
                {
                    Ok(message) => RunEvent::Done { message },
                    Err(error) => RunEvent::Error {
                        message: format!("failed to save reply: {error}"),
                    },
                },
                AgentEvent::Cancelled => RunEvent::Cancelled,
                AgentEvent::Error(message) => RunEvent::Error { message },
            };
            Some((sse, (store, session_id, display_anchor, events, last_usage)))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::UserRole;

    #[test]
    fn empty_tool_turn_persists_wire_calls() {
        futures::executor::block_on(async {
            let store = Arc::new(Store::new(AppDb::open_in_memory().unwrap()));
            store.migrate().await.unwrap();
            let user = store
                .insert_user("u", "hash", UserRole::Admin, 1)
                .await
                .unwrap();
            let session = store
                .create_session("s", None, None, None, user.id, 1)
                .await
                .unwrap();
            let calls = vec![openwebide_core::ToolCall {
                id: "wire-id".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
            }];
            let events = map_agent_events(
                store.clone(),
                session.id,
                7,
                stream::iter([AgentEvent::TurnCalls {
                    text: String::new(),
                    calls: calls.clone(),
                }]),
            )
            .collect::<Vec<_>>()
            .await;
            let RunEvent::Interim { message } = &events[0] else {
                panic!("missing interim")
            };
            assert!(message.content.is_empty());
            assert_eq!(message.tool_calls.as_ref(), Some(&calls));
            assert_eq!(
                store.list_messages(session.id).await.unwrap(),
                vec![message.clone()]
            );
        });
    }

    #[test]
    fn maps_interim_text_usage_and_display_anchors() {
        futures::executor::block_on(async {
            let store = Arc::new(Store::new(AppDb::open_in_memory().unwrap()));
            store.migrate_with(&|_| true).await.unwrap();
            let user = store
                .insert_user("alice", "hash", UserRole::Admin, 1)
                .await
                .unwrap();
            let session = store
                .create_session("s", None, None, None, user.id, 1)
                .await
                .unwrap();
            let user_message = store
                .insert_message(session.id, Role::User, "go", 2)
                .await
                .unwrap();
            let anchor_id = user_message.id;
            let before = format!("a{anchor_id}t1c0");
            let after = format!("a{anchor_id}t2c0");
            let first_usage = TurnTelemetry {
                prompt_tokens: 10,
                completion_tokens: 2,
                ..Default::default()
            };
            let last_usage = TurnTelemetry {
                prompt_tokens: 20,
                completion_tokens: 3,
                ..Default::default()
            };
            let events = vec![
                AgentEvent::ToolCall {
                    id: before.clone(),
                    name: "read_file".into(),
                    summary: "first".into(),
                },
                AgentEvent::TextDelta("checking".into()),
                AgentEvent::Telemetry(first_usage),
                AgentEvent::TurnCalls {
                    text: "checking".into(),
                    calls: vec![],
                },
                AgentEvent::PermissionRequest {
                    id: after.clone(),
                    name: "write_file".into(),
                    summary: "second".into(),
                },
                AgentEvent::ToolCall {
                    id: after.clone(),
                    name: "write_file".into(),
                    summary: "second".into(),
                },
                AgentEvent::ToolResult {
                    id: after.clone(),
                    name: "write_file".into(),
                    ok: true,
                    summary: "wrote".into(),
                    diff: None,
                },
                AgentEvent::TextDelta("done".into()),
                AgentEvent::Telemetry(last_usage),
                AgentEvent::FinalText("done".into()),
            ];
            let mapped =
                map_agent_events(store.clone(), session.id, anchor_id, stream::iter(events))
                    .collect::<Vec<_>>()
                    .await;
            assert!(matches!(&mapped[1], RunEvent::Delta { content: text } if text == "checking"));
            assert!(matches!(&mapped[2], RunEvent::Telemetry { usage } if *usage == first_usage));
            let RunEvent::Interim { message: interim } = &mapped[3] else {
                panic!("missing interim")
            };
            assert_eq!(interim.content, "checking");
            assert_eq!(interim.usage, Some(first_usage));
            assert!(matches!(&mapped[4], RunEvent::PermissionRequest { id, .. } if id == &after));
            let steps = store.list_tool_steps(session.id).await.unwrap();
            assert_eq!(steps[0].tool_call_id, before);
            assert_eq!(steps[0].anchor_message_id, anchor_id);
            assert_eq!(steps[1].tool_call_id, after);
            assert_eq!(steps[1].anchor_message_id, interim.id);
            assert_eq!(steps[1].ok, Some(true));
            let RunEvent::Done {
                message: final_message,
            } = mapped.last().unwrap()
            else {
                panic!("missing final")
            };
            assert_eq!(final_message.usage, Some(last_usage));
            let messages = store.list_messages(session.id).await.unwrap();
            assert_eq!(
                messages,
                vec![user_message, interim.clone(), final_message.clone()]
            );
        });
    }

    #[test]
    fn truncated_reply_does_not_inherit_tool_turn_usage() {
        futures::executor::block_on(async {
            for (has_text, denied) in [(true, false), (false, false), (true, true), (false, true)] {
                let store = Arc::new(Store::new(AppDb::open_in_memory().unwrap()));
                store.migrate_with(&|_| true).await.unwrap();
                let user = store
                    .insert_user("alice", "hash", UserRole::Admin, 1)
                    .await
                    .unwrap();
                let session = store
                    .create_session("s", None, None, None, user.id, 1)
                    .await
                    .unwrap();
                let usage = TurnTelemetry {
                    prompt_tokens: 100,
                    completion_tokens: 20,
                    ..Default::default()
                };
                let mut events = vec![AgentEvent::Telemetry(usage)];
                if has_text {
                    events.push(AgentEvent::TurnCalls {
                        text: "checking".into(),
                        calls: vec![],
                    });
                }
                if denied {
                    events.push(AgentEvent::PermissionRequest {
                        id: "a7t1c0".into(),
                        name: "write_file".into(),
                        summary: "write".into(),
                    });
                    events.push(AgentEvent::ToolResult {
                        id: "a7t1c0".into(),
                        name: "write_file".into(),
                        ok: false,
                        summary: "denied".into(),
                        diff: None,
                    });
                } else {
                    events.push(AgentEvent::ToolCall {
                        id: "a7t1c0".into(),
                        name: "read_file".into(),
                        summary: "read".into(),
                    });
                }
                let partial = format!("partial{}", openwebide_core::REPLY_TRUNCATED_MARKER);
                events.push(AgentEvent::TextDelta("partial".into()));
                events.push(AgentEvent::FinalText(partial.clone()));
                let mapped = map_agent_events(store.clone(), session.id, 7, stream::iter(events))
                    .collect::<Vec<_>>()
                    .await;
                let RunEvent::Done { message: reply } = mapped.last().unwrap() else {
                    panic!("missing final")
                };
                assert_eq!(reply.content, partial);
                assert_eq!(reply.usage, None);
                let messages = store.list_messages(session.id).await.unwrap();
                assert_eq!(messages.last(), Some(reply));
                if has_text {
                    assert_eq!(messages[0].usage, Some(usage));
                }
                assert_eq!(messages.len(), if has_text { 2 } else { 1 });
            }
        });
    }

    #[test]
    fn interim_persistence_failure_still_forwards_text() {
        futures::executor::block_on(async {
            let store = Arc::new(Store::new(AppDb::open_in_memory().unwrap()));
            store.migrate_with(&|_| true).await.unwrap();
            let mapped = map_agent_events(
                store,
                999,
                7,
                stream::iter([AgentEvent::TurnCalls {
                    text: "checking".into(),
                    calls: vec![],
                }]),
            )
            .collect::<Vec<_>>()
            .await;
            assert!(
                matches!(&mapped[0], RunEvent::Interim { message } if message.id == 0 && message.content == "checking")
            );
        });
    }
}
