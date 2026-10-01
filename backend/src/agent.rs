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
/// boundaries and during interruptible tools.
pub struct CancelFlag {
    store: Arc<Store<AppDb>>,
    session_id: i64,
    started_ms: i64,
    poll_interval: Duration,
}

impl CancelFlag {
    pub fn new(store: Arc<Store<AppDb>>, session_id: i64, started_ms: i64) -> Self {
        Self {
            store,
            session_id,
            started_ms,
            poll_interval: Duration::from_millis(250),
        }
    }
}

impl CancelCheck for CancelFlag {
    async fn cancelled(&self) {
        while !self.check().await {
            if self.poll_interval.is_zero() {
                let mut yielded = false;
                futures::future::poll_fn(|cx| {
                    if yielded {
                        std::task::Poll::Ready(())
                    } else {
                        yielded = true;
                        cx.waker().wake_by_ref();
                        std::task::Poll::Pending
                    }
                })
                .await;
            } else {
                spin_sdk::time::sleep(self.poll_interval).await;
            }
        }
    }

    fn check(&self) -> impl Future<Output = bool> + Send {
        let store = self.store.clone();
        let session_id = self.session_id;
        let started_ms = self.started_ms;
        async move {
            match store.cancel_requested_since(session_id, started_ms).await {
                Ok(cancelled) => cancelled,
                Err(error) => {
                    eprintln!("session {session_id}: cancel_requested_since: {error}");
                    false
                }
            }
        }
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
    started_ms: i64,
    poll_interval: Duration,
    timeout: Duration,
}

impl PermissionPoller {
    pub fn new(store: Arc<Store<AppDb>>, session_id: i64, started_ms: i64) -> Self {
        Self {
            store,
            session_id,
            started_ms,
            poll_interval: PERMISSION_POLL_INTERVAL,
            timeout: PERMISSION_TIMEOUT,
        }
    }
}

impl PermissionGate for PermissionPoller {
    fn approve(&self, call: &ToolCall) -> impl Future<Output = bool> + Send {
        let store = self.store.clone();
        let session_id = self.session_id;
        let started_ms = self.started_ms;
        let tool_call_id = call.id.clone();
        let poll_interval = self.poll_interval;
        let timeout = self.timeout;
        async move {
            let started = Instant::now();
            loop {
                match store.take_tool_permission(session_id, &tool_call_id).await {
                    Ok(Some(decision)) => return decision,
                    Ok(None) => {}
                    Err(error) => eprintln!(
                        "session {session_id}: take_tool_permission {tool_call_id}: {error}"
                    ),
                }
                // A cancel landing while waiting also denies the call; the
                // loop re-checks the cancel flag and reports `Cancelled`.
                match store.cancel_requested_since(session_id, started_ms).await {
                    Ok(true) => return false,
                    Ok(false) => {}
                    Err(error) => {
                        eprintln!("session {session_id}: cancel_requested_since: {error}")
                    }
                }
                if started.elapsed() >= timeout {
                    return false;
                }
                if !poll_interval.is_zero() {
                    spin_sdk::time::sleep(poll_interval).await;
                }
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
        (
            store,
            session_id,
            display_anchor,
            events,
            last_usage,
            String::new(),
        ),
        move |state| async move {
            let (store, session_id, mut display_anchor, mut events, mut last_usage, mut reasoning) =
                state;
            let Some(event) = events.next().await else {
                if let Err(error) = store
                    .clear_tool_permissions_for_run(session_id, anchor_id)
                    .await
                {
                    eprintln!("session {session_id}: clear_tool_permissions_for_run: {error}");
                }
                return None;
            };
            let sse = match event {
                AgentEvent::ToolCall { id, name, summary } => {
                    last_usage = None;
                    if let Err(error) = store
                        .upsert_tool_step(
                            session_id,
                            display_anchor,
                            &id,
                            &name,
                            &summary,
                            now(),
                            None,
                        )
                        .await
                    {
                        eprintln!("session {session_id}: upsert_tool_step: {error}");
                    }
                    RunEvent::ToolCall { id, name, summary }
                }
                AgentEvent::PermissionRequest {
                    id,
                    name,
                    summary,
                    diff,
                    note,
                } => {
                    last_usage = None;
                    if let Err(error) = store
                        .upsert_tool_step(
                            session_id,
                            display_anchor,
                            &id,
                            &name,
                            &summary,
                            now(),
                            diff.as_ref(),
                        )
                        .await
                    {
                        eprintln!("session {session_id}: upsert_tool_step: {error}");
                    }
                    RunEvent::PermissionRequest {
                        id,
                        name,
                        summary,
                        diff,
                        note,
                    }
                }
                AgentEvent::ToolResult {
                    id,
                    name,
                    ok,
                    summary,
                    diff,
                } => {
                    if let Err(error) = store
                        .complete_tool_step(session_id, &id, ok, &summary, diff.as_ref())
                        .await
                    {
                        eprintln!("session {session_id}: complete_tool_step: {error}");
                    }
                    RunEvent::ToolResult {
                        id,
                        name,
                        ok,
                        summary,
                        diff,
                    }
                }
                AgentEvent::ReasoningDelta(content) => {
                    reasoning.push_str(&content);
                    RunEvent::ReasoningDelta { content }
                }
                AgentEvent::TextDelta(delta) => RunEvent::Delta { content: delta },
                AgentEvent::TurnCalls { text, calls } => {
                    let text =
                        openwebide_core::with_reasoning(&std::mem::take(&mut reasoning), &text);
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
                AgentEvent::FinalText(text) => {
                    let text = openwebide_core::with_reasoning(&reasoning, &text);
                    match store
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
                    }
                }
                AgentEvent::Cancelled => RunEvent::Cancelled,
                AgentEvent::Error(message) => RunEvent::Error { message },
            };
            Some((
                sse,
                (
                    store,
                    session_id,
                    display_anchor,
                    events,
                    last_usage,
                    reasoning,
                ),
            ))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::UserRole;

    use openwebide_agent::{ToolExecutor, ToolOutcome};
    use openwebide_core::{ChatCompletion, ChatResponse, ModelInfo, ProviderKind};
    use openwebide_llm::{LlmProvider, ProviderError, StreamChunk, ToolStreamChunk};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct ScriptedProvider(Mutex<VecDeque<ChatResponse>>);

    impl LlmProvider for ScriptedProvider {
        fn kind(&self) -> ProviderKind {
            ProviderKind::Ollama
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            Ok(vec![])
        }
        async fn chat(&self, _: &ChatRequest) -> Result<String, ProviderError> {
            unreachable!()
        }
        fn chat_stream(
            &self,
            _: &ChatRequest,
        ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
            unreachable!()
        }
        async fn chat_tools(&self, _: &ChatRequest) -> Result<ChatCompletion, ProviderError> {
            unreachable!()
        }
        fn chat_tools_stream(
            &self,
            _: &ChatRequest,
        ) -> Pin<Box<dyn Stream<Item = Result<ToolStreamChunk, ProviderError>> + Send>> {
            let response = self.0.lock().unwrap().pop_front().unwrap();
            Box::pin(stream::iter([Ok(ToolStreamChunk::Response(response))]))
        }
        async fn context_limit(&self, _: Option<&str>) -> Result<Option<usize>, ProviderError> {
            Ok(None)
        }
    }

    struct RecordingExecutor(Arc<Mutex<Vec<String>>>);

    impl ToolExecutor for RecordingExecutor {
        fn describe(&self, call: &ToolCall) -> String {
            call.name.clone()
        }
        async fn execute(&self, call: &ToolCall) -> ToolOutcome {
            self.0.lock().unwrap().push(call.name.clone());
            ToolOutcome {
                ok: true,
                content: "ok".into(),
                summary: "ok".into(),
                diff: None,
            }
        }
    }

    fn tool_turn(name: &str) -> ChatResponse {
        ChatResponse::ToolCalls(vec![ToolCall {
            id: "call_0".into(),
            name: name.into(),
            arguments: "{}".into(),
        }])
    }

    fn request() -> ChatRequest {
        ChatRequest {
            connection_id: 1,
            system_prompt: None,
            model: None,
            messages: vec![],
            tools: vec![],
        }
    }

    async fn test_store() -> (Arc<Store<AppDb>>, i64) {
        let store = Arc::new(Store::new(AppDb::open_in_memory().unwrap()));
        store.migrate().await.unwrap();
        let user = store
            .insert_user("tester", "hash", UserRole::Admin, 1)
            .await
            .unwrap();
        let session = store
            .create_session("s", None, None, None, user.id, 1)
            .await
            .unwrap();
        (store, session.id)
    }

    fn test_gate(store: Arc<Store<AppDb>>, session_id: i64, started_ms: i64) -> PermissionPoller {
        PermissionPoller {
            store,
            session_id,
            started_ms,
            poll_interval: Duration::ZERO,
            timeout: Duration::from_millis(20),
        }
    }

    #[test]
    fn repeated_provider_ids_do_not_reuse_approval() {
        futures::executor::block_on(async {
            let (store, session_id) = test_store().await;
            let executed = Arc::new(Mutex::new(vec![]));
            let events = openwebide_agent::run(
                ScriptedProvider(Mutex::new(VecDeque::from([
                    tool_turn("write_file"),
                    tool_turn("run_command"),
                    ChatResponse::Text("done".into()),
                ]))),
                RecordingExecutor(executed.clone()),
                request(),
                AgentConfig::default(),
                CancelFlag::new(store.clone(), session_id, 1000),
                test_gate(store.clone(), session_id, 1000),
                7,
            );
            let mut events = Box::pin(events);
            let mut prompts = vec![];
            let mut denied = None;
            while let Some(event) = events.next().await {
                match event {
                    AgentEvent::PermissionRequest { id, .. } => {
                        prompts.push(id.clone());
                        if prompts.len() == 1 {
                            store
                                .set_tool_permission(session_id, &id, true)
                                .await
                                .unwrap();
                        }
                    }
                    AgentEvent::ToolResult {
                        id,
                        ok: false,
                        summary,
                        ..
                    } => denied = Some((id, summary)),
                    _ => {}
                }
            }
            assert_eq!(prompts, ["a7t1c0", "a7t2c0"]);
            assert_eq!(*executed.lock().unwrap(), ["write_file"]);
            assert_eq!(denied, Some(("a7t2c0".into(), "denied by user".into())));
        });
    }

    #[test]
    fn cleanup_preserves_other_run_and_session_permissions() {
        futures::executor::block_on(async {
            let (store, session_id) = test_store().await;
            let other = store
                .create_session(
                    "other",
                    None,
                    None,
                    None,
                    openwebide_core::UserId::new(1),
                    1,
                )
                .await
                .unwrap();
            for (session, id) in [
                (session_id, "a7t1c0"),
                (session_id, "a7t2c0"),
                (session_id, "a70t1c0"),
                (other.id, "a7t1c0"),
            ] {
                store.set_tool_permission(session, id, true).await.unwrap();
            }
            let events = [
                AgentEvent::TurnCalls {
                    text: "interim".into(),
                    calls: vec![],
                },
                AgentEvent::Cancelled,
            ];
            map_agent_events(store.clone(), session_id, 7, stream::iter(events))
                .collect::<Vec<_>>()
                .await;
            for id in ["a7t1c0", "a7t2c0"] {
                assert_eq!(
                    store.take_tool_permission(session_id, id).await.unwrap(),
                    None
                );
            }
            assert_eq!(
                store
                    .take_tool_permission(session_id, "a70t1c0")
                    .await
                    .unwrap(),
                Some(true)
            );
            assert_eq!(
                store
                    .take_tool_permission(other.id, "a7t1c0")
                    .await
                    .unwrap(),
                Some(true)
            );
        });
    }

    #[test]
    fn cancel_only_applies_after_run_start() {
        futures::executor::block_on(async {
            let (store, session_id) = test_store().await;
            let cancel = CancelFlag::new(store.clone(), session_id, 2000);
            store.request_cancel(session_id, 1000).await.unwrap();
            assert!(!cancel.check().await);
            store.request_cancel(session_id, 3000).await.unwrap();
            assert!(cancel.check().await);
        });
    }

    #[test]
    fn stop_then_resend_keeps_waiting_run_cancelled() {
        futures::executor::block_on(async {
            let (store, session_id) = test_store().await;
            let executed = Arc::new(Mutex::new(vec![]));
            let gate = test_gate(store.clone(), session_id, 1000);
            let mut run_a = Box::pin(openwebide_agent::run(
                ScriptedProvider(Mutex::new(VecDeque::from([tool_turn("write_file")]))),
                RecordingExecutor(executed.clone()),
                request(),
                AgentConfig::default(),
                CancelFlag::new(store.clone(), session_id, 1000),
                test_gate(store.clone(), session_id, 1000),
                7,
            ));
            assert!(matches!(
                run_a.next().await,
                Some(AgentEvent::TurnCalls { .. })
            ));
            let Some(AgentEvent::PermissionRequest { id, name, .. }) = run_a.next().await else {
                panic!("missing permission prompt")
            };
            assert_eq!(
                store.take_tool_permission(session_id, &id).await.unwrap(),
                None
            );
            store.request_cancel(session_id, 1500).await.unwrap();
            let cancel_b = CancelFlag::new(store.clone(), session_id, 1600);
            let _gate_b = test_gate(store.clone(), session_id, 1600);
            assert!(
                !gate
                    .approve(&ToolCall {
                        id,
                        name,
                        arguments: "{}".into()
                    })
                    .await
            );
            assert_eq!(run_a.next().await, Some(AgentEvent::Cancelled));
            assert_eq!(run_a.next().await, None);
            assert!(!cancel_b.check().await);
            assert!(executed.lock().unwrap().is_empty());
        });
    }

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
    fn maps_and_persists_the_awaiting_preview_then_replaces_it_with_the_result() {
        futures::executor::block_on(async {
            let store = Arc::new(Store::new(AppDb::open_in_memory().unwrap()));
            store.migrate_with(&|_| true).await.unwrap();
            let user = store
                .insert_user("preview", "hash", UserRole::Admin, 1)
                .await
                .unwrap();
            let session = store
                .create_session("preview", None, None, None, user.id, 1)
                .await
                .unwrap();
            let anchor = store
                .insert_message(session.id, Role::User, "write", 2)
                .await
                .unwrap();
            let mut diff = openwebide_core::FileDiff {
                path: "file".into(),
                old: Some("before".into()),
                new: "after".into(),
                old_unavailable: false,
                backup_path: None,
            };
            let permission = AgentEvent::PermissionRequest {
                id: "a1t1c0".into(),
                name: "write_file".into(),
                summary: "write file".into(),
                diff: Some(diff.clone()),
                note: Some("preview".into()),
            };
            diff.old = Some("changed before execution".into());
            let result = AgentEvent::ToolResult {
                id: "a1t1c0".into(),
                name: "write_file".into(),
                ok: true,
                summary: "written".into(),
                diff: Some(diff.clone()),
            };
            let events = map_agent_events(
                store.clone(),
                session.id,
                anchor.id,
                stream::iter([permission, result]),
            );
            futures::pin_mut!(events);
            let Some(RunEvent::PermissionRequest {
                diff: preview,
                note,
                ..
            }) = events.next().await
            else {
                panic!("missing permission")
            };
            assert_eq!(note.as_deref(), Some("preview"));
            let steps = store.list_tool_steps(session.id).await.unwrap();
            assert_eq!(steps[0].diff, preview);
            assert_eq!(steps[0].ok, None);
            assert_eq!(steps[0].result_summary, None);
            events.next().await.unwrap();
            let steps = store.list_tool_steps(session.id).await.unwrap();
            assert_eq!(steps[0].diff, Some(diff));
            assert_eq!(steps[0].ok, Some(true));
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
                    diff: None,
                    note: None,
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
                        diff: None,
                        note: None,
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
    #[test]
    fn cancellation_waiter_observes_request() {
        futures::executor::block_on(async {
            let (store, session_id) = test_store().await;
            let mut cancel = CancelFlag::new(store.clone(), session_id, 2000);
            cancel.poll_interval = Duration::ZERO;
            store.request_cancel(session_id, 1000).await.unwrap();
            let mut waiter = Box::pin(cancel.cancelled());
            assert!(futures::poll!(&mut waiter).is_pending());
            store.request_cancel(session_id, 3000).await.unwrap();
            waiter.await;
        });
    }
    #[test]
    fn reasoning_prefix_is_persisted_per_turn() {
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
            let answer = format!("answer{}", openwebide_core::REPLY_CUT_OFF_MARKER);
            let events = map_agent_events(
                store.clone(),
                session.id,
                7,
                stream::iter([
                    AgentEvent::ReasoningDelta("first".into()),
                    AgentEvent::TurnCalls {
                        text: "checking".into(),
                        calls: vec![],
                    },
                    AgentEvent::ReasoningDelta("r".into()),
                    AgentEvent::TextDelta("answer".into()),
                    AgentEvent::FinalText(answer.clone()),
                ]),
            )
            .collect::<Vec<_>>()
            .await;
            assert!(
                matches!(&events[0], RunEvent::ReasoningDelta { content } if content == "first")
            );
            let messages = store.list_messages(session.id).await.unwrap();
            assert_eq!(messages[0].content, "<think>first</think>checking");
            assert_eq!(messages[1].content, format!("<think>r</think>{answer}"));
        });
    }
}
