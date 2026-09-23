//! Agentic coding for remote-mode projects: the backend's `ToolExecutor`
//! (workspace-confined file tools) and the SSE stream that wraps the agent
//! loop from the `openwebide-agent` crate.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::files::HostFsVfs;
use crate::http_client::SpinHttpClient;
use crate::sse::SseEvent;
use crate::state::now;
use futures::{Stream, StreamExt, stream};
use openwebide_agent::{AgentConfig, AgentEvent, CancelCheck, PermissionGate};
use openwebide_agent::{VfsToolExecutor, vfs_tools};
use openwebide_core::{ChatMessage, ChatRequest, Role, ToolCall, ToolDefinition, TurnTelemetry};
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
) -> Pin<Box<dyn Stream<Item = SseEvent> + Send + 'static>> {
    let executor = VfsToolExecutor::with_web_and_bridge(
        HostFsVfs::new(base),
        crate::web::SpinWebClient,
        crate::bridge_client::SpinBridgeClient::default(),
    );
    // Tool steps anchor to the user message that started this turn, so a
    // reloaded session renders them right after it; the same id namespaces
    // the run's tool-call ids.
    let anchor = user_message.id;
    let events = openwebide_agent::run(provider, executor, request, config, cancel, gate, anchor);
    let last_usage: Option<TurnTelemetry> = None;
    let tail = stream::unfold(
        (store, session_id, anchor, events, last_usage),
        |state| async move {
            let (store, session_id, anchor, mut events, mut last_usage) = state;
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
                    let _ = store
                        .upsert_tool_step(session_id, anchor, &id, &name, &summary, now())
                        .await;
                    SseEvent::ToolCall { id, name, summary }
                }
                AgentEvent::PermissionRequest { id, name, summary } => {
                    let _ = store
                        .upsert_tool_step(session_id, anchor, &id, &name, &summary, now())
                        .await;
                    SseEvent::PermissionRequest { id, name, summary }
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
                    SseEvent::ToolResult {
                        id,
                        name,
                        ok,
                        summary,
                        diff,
                    }
                }
                AgentEvent::Telemetry(usage) => {
                    last_usage = Some(usage);
                    SseEvent::Telemetry(usage)
                }
                AgentEvent::FinalText(text) => match store
                    .insert_message_with_usage(
                        session_id,
                        Role::Assistant,
                        &text,
                        now(),
                        last_usage.as_ref(),
                    )
                    .await
                {
                    Ok(message) => SseEvent::Done(message),
                    Err(error) => SseEvent::Error(format!("failed to save reply: {error}")),
                },
                AgentEvent::Cancelled => SseEvent::Cancelled,
                AgentEvent::Error(message) => SseEvent::Error(message),
            };
            Some((sse, (store, session_id, anchor, events, last_usage)))
        },
    );
    Box::pin(stream::iter([SseEvent::Message(user_message)]).chain(tail))
}
