use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{Stream, StreamExt};
use openwebide_agent::{AgentConfig, AgentEvent, VfsToolExecutor};
use openwebide_core::{
    BridgeServerMessage, ChatMessage, EditorContext, REPLY_TRUNCATED_MARKER, Role, RunEvent,
    RunInfo, RunKind, RunPlan, RunRejectCode, RunSnapshot, TurnTelemetry,
};
use openwebide_llm::{LlmProvider, ProviderError, StreamChunk};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::auth::Principal;
use crate::runs::agent_host::{BackendWebClient, BridgeCancel, BridgeGate, InProcessBridgeClient};
use crate::runs::backend_client::RunBackend;
use crate::runs::native_vfs::NativeFsVfs;
use crate::server::WriterCmd;
use crate::terminals::seq_ring::SeqRing;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct Delivery {
    ring: SeqRing<RunEvent>,
    snapshot: RunSnapshot,
    seq: u64,
}

pub struct Run {
    pub run_id: String,
    owner: i64,
    session_id: i64,
    started_at: u64,
    running: AtomicBool,
    finished_at: Mutex<Option<Instant>>,
    delivery: Mutex<Delivery>,
    pub cancel: BridgeCancel,
    pub gate: BridgeGate,
}

impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Run").field("run_id", &self.run_id).finish()
    }
}

impl Run {
    fn new(run_id: String, owner: i64, session_id: i64, capacity: usize) -> Self {
        let cancel = BridgeCancel::default();
        Self {
            run_id,
            owner,
            session_id,
            started_at: now(),
            running: AtomicBool::new(true),
            finished_at: Mutex::new(None),
            delivery: Mutex::new(Delivery {
                ring: SeqRing::new(None, Some(capacity)),
                snapshot: RunSnapshot::default(),
                seq: 0,
            }),
            gate: BridgeGate::new(cancel.clone()),
            cancel,
        }
    }
    pub fn emit(&self, event: RunEvent) {
        let finished = matches!(
            event,
            RunEvent::Done { .. } | RunEvent::Cancelled | RunEvent::Error { .. }
        );
        let mut delivery = self.delivery.lock().unwrap();
        delivery.snapshot.apply(&event);
        delivery.seq = delivery.ring.push(|_| event, 0);
        if finished {
            self.gate.clear();
            *self.finished_at.lock().unwrap() = Some(Instant::now());
            self.running.store(false, Ordering::SeqCst);
        }
    }
    fn snapshot_message(&self, delivery: &Delivery) -> BridgeServerMessage {
        BridgeServerMessage::RunSnapshot {
            run_id: self.run_id.clone(),
            session_id: self.session_id,
            seq: delivery.seq,
            snapshot: delivery.snapshot.clone(),
        }
    }
    pub(crate) async fn forward(
        self: Arc<Self>,
        last_seq: Option<u64>,
        sender: mpsc::Sender<WriterCmd>,
    ) {
        let mut watch = self.delivery.lock().unwrap().ring.subscribe();
        let mut cursor = last_seq;
        loop {
            let (messages, next, finished) = {
                let delivery = self.delivery.lock().unwrap();
                let (batch, lag) = delivery.ring.read_after(cursor.unwrap_or(0), 4096);
                if cursor.is_none() || lag.is_some() || cursor.is_some_and(|c| c > delivery.seq) {
                    (
                        vec![self.snapshot_message(&delivery)],
                        delivery.seq,
                        delivery.snapshot.finished.is_some(),
                    )
                } else {
                    let next = batch
                        .last()
                        .map(|(seq, _)| *seq)
                        .unwrap_or(cursor.unwrap_or(0));
                    (
                        batch
                            .into_iter()
                            .map(|(seq, event)| BridgeServerMessage::RunEvent {
                                run_id: self.run_id.clone(),
                                seq,
                                event,
                            })
                            .collect(),
                        next,
                        delivery.snapshot.finished.is_some() && next == delivery.seq,
                    )
                }
            };
            for message in messages {
                if sender.send(WriterCmd::send(message)).await.is_err() {
                    return;
                }
            }
            cursor = Some(next);
            if finished {
                return;
            }
            if watch.changed().await.is_err() {
                return;
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct RunRegistry {
    runs: Mutex<HashMap<String, Arc<Run>>>,
}

pub struct StartRun {
    pub run_id: String,
    pub session_id: i64,
    pub content: String,
    pub model: Option<String>,
    pub editor_context: Option<EditorContext>,
}

impl RunRegistry {
    pub fn get(&self, principal: &Principal, run_id: &str) -> Result<Arc<Run>, String> {
        let runs = self.runs.lock().unwrap();
        runs.get(run_id)
            .filter(|run| matches!(principal, Principal::User { user_id } if *user_id == run.owner))
            .cloned()
            .ok_or_else(|| "run not found".into())
    }
    pub fn list(&self, principal: &Principal, session_id: i64) -> Vec<RunInfo> {
        self.runs
            .lock()
            .unwrap()
            .values()
            .filter(|run| {
                run.session_id == session_id
                    && matches!(principal, Principal::User { user_id } if *user_id == run.owner)
            })
            .map(|run| RunInfo {
                run_id: run.run_id.clone(),
                session_id,
                running: run.running.load(Ordering::SeqCst),
                seq: run.delivery.lock().unwrap().seq,
                started_at: run.started_at,
            })
            .collect()
    }
    pub fn reap(&self) {
        self.runs.lock().unwrap().retain(|_, run| {
            run.finished_at
                .lock()
                .unwrap()
                .is_none_or(|at| at.elapsed() < Duration::from_secs(600))
        });
    }
    pub async fn start<B, P, F>(
        &self,
        principal: &Principal,
        start: StartRun,
        workspace: &Path,
        backend: Arc<B>,
        provider: F,
    ) -> Result<Arc<Run>, (RunRejectCode, String)>
    where
        B: RunBackend + 'static,
        P: LlmProvider + 'static,
        F: FnOnce(&RunPlan) -> P,
    {
        let run = self.reserve(principal, &start)?;
        self.prepare(
            run,
            start,
            workspace,
            backend,
            provider,
            Arc::new(crate::exec::HostExecution),
        )
        .await
    }

    pub(crate) fn reserve(
        &self,
        principal: &Principal,
        start: &StartRun,
    ) -> Result<Arc<Run>, (RunRejectCode, String)> {
        let Principal::User { user_id } = principal else {
            return Err((RunRejectCode::Unauthorized, "unauthorized".into()));
        };
        let run = Arc::new(Run::new(
            start.run_id.clone(),
            *user_id,
            start.session_id,
            4096,
        ));
        {
            let mut runs = self.runs.lock().unwrap();
            if runs.contains_key(&start.run_id)
                || runs
                    .values()
                    .any(|r| r.session_id == start.session_id && r.running.load(Ordering::SeqCst))
            {
                return Err((
                    RunRejectCode::Busy,
                    "session already has an active run".into(),
                ));
            }
            runs.insert(start.run_id.clone(), run.clone());
        }
        Ok(run)
    }

    pub(crate) async fn prepare<B, P, F>(
        &self,
        run: Arc<Run>,
        start: StartRun,
        workspace: &Path,
        backend: Arc<B>,
        provider: F,
        execution: Arc<dyn crate::exec::ToolExecution>,
    ) -> Result<Arc<Run>, (RunRejectCode, String)>
    where
        B: RunBackend + 'static,
        P: LlmProvider + 'static,
        F: FnOnce(&RunPlan) -> P,
    {
        let user_id = run.owner;
        if run.cancel.is_cancelled() {
            run.emit(RunEvent::Cancelled);
            return Ok(run);
        }
        let prepared = async {
            let mut plan = backend
                .run_plan(
                    user_id,
                    start.session_id,
                    &start.content,
                    start.model.as_deref(),
                    start.editor_context.as_ref(),
                )
                .await
                .map_err(|e| (RunRejectCode::PlanFailed, e))?;
            let dir = match &plan.kind {
                RunKind::Chat => None,
                RunKind::Agent { project_path } => Some(
                    crate::paths::resolve_in_root(workspace, Some(project_path))
                        .map_err(|e| (RunRejectCode::ProjectUnavailable, e))?,
                ),
            };
            let message = backend
                .persist_message(
                    user_id,
                    start.session_id,
                    Role::User,
                    &plan.user_content,
                    None,
                    None,
                )
                .await
                .map_err(|e| (RunRejectCode::PlanFailed, e))?;
            plan.request.messages.push(message.clone());
            Ok((plan, dir, message))
        }
        .await;
        let (plan, dir, message) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.runs.lock().unwrap().remove(&start.run_id);
                return Err(error);
            }
        };
        let provider = provider(&plan);
        run.emit(RunEvent::Message {
            message: message.clone(),
        });
        let body_run = run.clone();
        tokio::spawn(async move {
            run_body(
                body_run, provider, backend, plan, dir, message.id, execution,
            )
            .await;
        });
        Ok(run)
    }
}

#[tracing::instrument(skip_all, fields(run_id = %run.run_id))]
async fn run_body<B: RunBackend + 'static, P: LlmProvider + 'static>(
    run: Arc<Run>,
    provider: P,
    backend: Arc<B>,
    plan: RunPlan,
    dir: Option<PathBuf>,
    anchor_id: i64,
    execution: Arc<dyn crate::exec::ToolExecution>,
) {
    match plan.kind {
        RunKind::Chat => {
            let mut stream = provider.chat_stream(&plan.request);
            let mut text = String::new();
            let mut reasoning = String::new();
            let mut usage = None;
            loop {
                let chunk = tokio::select! {
                    () = run.cancel.cancelled() => { run.emit(RunEvent::Cancelled); return; }
                    chunk = stream.next() => chunk,
                };
                match chunk {
                    Some(Ok(StreamChunk::Delta(delta))) => {
                        text.push_str(&delta);
                        run.emit(RunEvent::Delta { content: delta });
                    }
                    Some(Ok(StreamChunk::Reasoning(content))) => {
                        reasoning.push_str(&content);
                        run.emit(RunEvent::ReasoningDelta { content });
                    }
                    Some(Ok(StreamChunk::Stop(reason))) => {
                        if reason == openwebide_core::StopReason::Length {
                            text.push_str(openwebide_core::REPLY_CUT_OFF_MARKER);
                        }
                    }
                    Some(Ok(StreamChunk::Usage(value))) => {
                        usage = Some(value);
                        run.emit(RunEvent::Telemetry { usage: value });
                    }
                    Some(Err(ProviderError::Incomplete))
                        if !text.is_empty() || !reasoning.is_empty() =>
                    {
                        text.push_str(REPLY_TRUNCATED_MARKER);
                        break;
                    }
                    Some(Err(error)) => {
                        run.emit(RunEvent::Error {
                            message: error.to_string(),
                        });
                        return;
                    }
                    None => break,
                }
            }
            if run.cancel.is_cancelled() {
                run.emit(RunEvent::Cancelled);
                return;
            }
            let text = openwebide_core::with_reasoning(&reasoning, &text);
            persist_final(&run, &*backend, &text, usage.as_ref()).await;
        }
        RunKind::Agent { .. } => {
            let dir = dir.expect("agent run has a resolved project");
            let executor = VfsToolExecutor::with_web_and_bridge(
                NativeFsVfs { root: dir.clone() },
                BackendWebClient {
                    backend: backend.clone(),
                    user_id: run.owner,
                },
                InProcessBridgeClient {
                    dir,
                    execution,
                    cancel: run.cancel.clone(),
                },
            );
            let memo = provider.tool_stream_memo();
            let connection_id = plan.connection.id;
            let tool_stream_revision = plan.connection.tool_stream_revision;
            let events = openwebide_agent::run(
                provider,
                executor,
                plan.request,
                AgentConfig::default(),
                run.cancel.clone(),
                run.gate.clone(),
                anchor_id,
            );
            let events = events.then(|event| async {
                if let Some(memo) = &memo {
                    record_tool_stream_memo(
                        &*backend,
                        run.owner,
                        connection_id,
                        tool_stream_revision,
                        memo,
                    )
                    .await;
                }
                event
            });
            map_agent_events(&run, &*backend, anchor_id, events).await;
        }
    }
}

pub(crate) async fn record_tool_stream_memo<B: RunBackend>(
    backend: &B,
    user_id: i64,
    connection_id: i64,
    tool_stream_revision: i64,
    memo: &openwebide_llm::ToolStreamMemo,
) {
    if memo.take_unrecorded()
        && let Err(error) = backend
            .set_tool_stream_unsupported(user_id, connection_id, tool_stream_revision)
            .await
    {
        tracing::warn!(%error, "failed to save streamed-tools memo");
    }
}

async fn persist_final<B: RunBackend>(
    run: &Run,
    backend: &B,
    text: &str,
    usage: Option<&TurnTelemetry>,
) {
    match backend
        .persist_message(
            run.owner,
            run.session_id,
            Role::Assistant,
            text,
            usage,
            None,
        )
        .await
    {
        Ok(message) => run.emit(RunEvent::Done { message }),
        Err(error) => run.emit(RunEvent::Error {
            message: format!("failed to save reply: {error}"),
        }),
    }
}

async fn map_agent_events<B: RunBackend>(
    run: &Run,
    backend: &B,
    anchor_id: i64,
    events: impl Stream<Item = AgentEvent> + Send,
) {
    let mut events = Box::pin(events);
    let mut display_anchor = anchor_id;
    let mut reasoning = String::new();
    let mut last_usage = None;
    while let Some(event) = events.next().await {
        let event = match event {
            AgentEvent::ReasoningDelta(content) => {
                reasoning.push_str(&content);
                RunEvent::ReasoningDelta { content }
            }
            AgentEvent::TextDelta(content) => RunEvent::Delta { content },
            AgentEvent::Telemetry(usage) => {
                last_usage = Some(usage);
                RunEvent::Telemetry { usage }
            }
            AgentEvent::TurnCalls { text, calls } => {
                let text = openwebide_core::with_reasoning(&std::mem::take(&mut reasoning), &text);
                let usage = last_usage.take();
                let message = match backend
                    .persist_message(
                        run.owner,
                        run.session_id,
                        Role::Assistant,
                        &text,
                        usage.as_ref(),
                        Some(&calls),
                    )
                    .await
                {
                    Ok(message) => {
                        display_anchor = message.id;
                        message
                    }
                    Err(_) => ChatMessage {
                        id: 0,
                        session_id: run.session_id,
                        role: Role::Assistant,
                        content: text,
                        created_at: i64::try_from(now()).unwrap_or(i64::MAX),
                        tool_calls: Some(calls),
                        tool_call_id: None,
                        usage,
                    },
                };
                RunEvent::Interim { message }
            }
            AgentEvent::ToolCall { id, name, summary } => {
                last_usage = None;
                let _ = backend
                    .upsert_tool_step(
                        run.owner,
                        run.session_id,
                        display_anchor,
                        &id,
                        &name,
                        &summary,
                        None,
                    )
                    .await;
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
                run.gate.prepare(&id);
                let _ = backend
                    .upsert_tool_step(
                        run.owner,
                        run.session_id,
                        display_anchor,
                        &id,
                        &name,
                        &summary,
                        diff.as_ref(),
                    )
                    .await;
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
                let _ = backend
                    .complete_tool_step(run.owner, run.session_id, &id, ok, &summary, diff.as_ref())
                    .await;
                RunEvent::ToolResult {
                    id,
                    name,
                    ok,
                    summary,
                    diff,
                }
            }
            AgentEvent::FinalText(text) => {
                let text = openwebide_core::with_reasoning(&reasoning, &text);
                persist_final(run, backend, &text, last_usage.as_ref()).await;
                return;
            }
            AgentEvent::Cancelled => RunEvent::Cancelled,
            AgentEvent::Error(message) => RunEvent::Error { message },
        };
        run.emit(event);
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
