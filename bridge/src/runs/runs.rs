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
    BridgeServerMessage, ChatMessage, EditorContext, Role, RunEvent, RunInfo, RunKind, RunPlan,
    RunRejectCode, RunSnapshot, TurnTelemetry,
};
use openwebide_llm::LlmProvider;
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
    pub queued_prompt: Option<openwebide_core::QueuedPromptKey>,
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
    pub async fn shutdown(&self) {
        let runs = self
            .runs
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for run in &runs {
            run.cancel.cancel();
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while runs.iter().any(|run| run.running.load(Ordering::SeqCst))
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
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
                RunKind::Chat | RunKind::WebChat => None,
                RunKind::Agent { project_path } => Some(
                    crate::paths::resolve_in_root(workspace, Some(project_path))
                        .map_err(|e| (RunRejectCode::ProjectUnavailable, e))?,
                ),
            };
            let message = if let Some(key) = start.queued_prompt {
                backend
                    .consume_queued_prompt(user_id, start.session_id, key, &plan.user_content)
                    .await
            } else {
                backend
                    .persist_message(
                        user_id,
                        start.session_id,
                        Role::User,
                        &plan.user_content,
                        None,
                        None,
                    )
                    .await
            }
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
    mut plan: RunPlan,
    dir: Option<PathBuf>,
    anchor_id: i64,
    execution: Arc<dyn crate::exec::ToolExecution>,
) {
    if plan.environment.timestamp == 0 {
        plan.environment.timestamp = i64::try_from(now()).unwrap_or(i64::MAX);
    }
    match plan.kind {
        RunKind::Chat => {
            let content =
                openwebide_agent::session::chat_context(&mut plan.request, &plan.environment);
            match backend
                .persist_message(
                    run.owner,
                    run.session_id,
                    Role::System,
                    &content,
                    None,
                    None,
                )
                .await
            {
                Ok(message) => run.emit(RunEvent::Message { message }),
                Err(_) => {
                    run.emit(RunEvent::Error {
                        message: "Could not save run context".into(),
                    });
                    return;
                }
            }
            let prepared = openwebide_agent::session::compact_request(
                &provider,
                &super::backend_client::ModelSource {
                    backend: backend.clone(),
                    user: run.owner,
                },
                &mut plan.request,
                &run.cancel,
                SessionPersistence {
                    run: &run,
                    backend: &*backend,
                },
                run.session_id,
                anchor_id,
            )
            .await;
            for event in prepared.events {
                run.emit(event);
            }
            if prepared.terminal {
                return;
            }
            let mut events = Box::pin(openwebide_agent::session::chat_events(
                SessionPersistence {
                    run: &run,
                    backend: &*backend,
                },
                provider.chat_stream(&plan.request),
                run.cancel.clone(),
                &plan.request,
            ));
            while let Some(event) = events.next().await {
                run.emit(event);
            }
        }
        RunKind::Agent { .. } | RunKind::WebChat => {
            let workspace = dir.map(|dir| {
                plan.environment.project_root = Some(dir.to_string_lossy().into_owned());
                plan.environment
                    .mode
                    .get_or_insert(openwebide_core::WorkspaceMode::Remote);
                VfsToolExecutor::with_web_and_bridge(
                    NativeFsVfs { root: dir.clone() },
                    BackendWebClient {
                        backend: backend.clone(),
                        user_id: run.owner,
                    },
                    InProcessBridgeClient {
                        dir,
                        execution: execution.clone(),
                        cancel: run.cancel.clone(),
                    },
                )
                .with_context(plan.environment.clone())
            });
            let executor = openwebide_agent::vfs_executor::SessionToolExecutor::new(
                workspace,
                BackendWebClient {
                    backend: backend.clone(),
                    user_id: run.owner,
                },
                plan.environment,
            )
            .with_host(crate::runs::agent_host::HostInfoClient(execution));
            let memo = provider.tool_stream_memo();
            let connection_id = plan.connection.id;
            let tool_stream_revision = plan.connection.tool_stream_revision;
            let memo_model = plan.connection.model.clone();
            let gate = openwebide_agent::policy::PolicyGate {
                manual: run.gate.clone(),
                source: super::backend_client::ApprovalAdapter {
                    backend: backend.clone(),
                    user: run.owner,
                    session: run.session_id,
                    connection_id,
                    model: plan.request.model.clone(),
                },
            };
            let events = openwebide_agent::run_with_compaction(
                provider,
                executor,
                plan.request,
                AgentConfig::default(),
                run.cancel.clone(),
                gate,
                anchor_id,
                super::backend_client::ModelSource {
                    backend: backend.clone(),
                    user: run.owner,
                },
            );
            let events = events.then(|event| async {
                if let Some(memo) = &memo {
                    record_tool_stream_memo(
                        &*backend,
                        run.owner,
                        connection_id,
                        tool_stream_revision,
                        memo_model.as_deref(),
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
    model: Option<&str>,
    memo: &openwebide_llm::ToolStreamMemo,
) {
    if memo.take_unrecorded()
        && let Err(error) = backend
            .set_tool_stream_unsupported(user_id, connection_id, tool_stream_revision, model)
            .await
    {
        tracing::warn!(%error, "failed to save streamed-tools memo");
    }
}

struct SessionPersistence<'a, B> {
    run: &'a Run,
    backend: &'a B,
}
impl<B: RunBackend> openwebide_agent::session::RunPersistence for SessionPersistence<'_, B> {
    fn now(&self) -> i64 {
        i64::try_from(now()).unwrap_or(i64::MAX)
    }
    async fn message(
        &self,
        role: Role,
        content: &str,
        usage: Option<&TurnTelemetry>,
        calls: Option<&[openwebide_core::ToolCall]>,
    ) -> Result<ChatMessage, String> {
        self.backend
            .persist_message(
                self.run.owner,
                self.run.session_id,
                role,
                content,
                usage,
                calls,
            )
            .await
    }
    async fn step(
        &self,
        anchor: i64,
        id: &str,
        name: &str,
        summary: &str,
        diff: Option<&openwebide_core::FileDiff>,
    ) -> Result<(), String> {
        self.backend
            .upsert_tool_step(
                self.run.owner,
                self.run.session_id,
                anchor,
                id,
                name,
                summary,
                diff,
            )
            .await
    }
    async fn checkpoint(
        &self,
        id: &str,
        checkpoint: &openwebide_core::rewind::ProjectCheckpoint,
    ) -> Result<(), String> {
        self.backend
            .save_project_checkpoint(self.run.owner, self.run.session_id, id, checkpoint)
            .await
    }
    async fn result(
        &self,
        id: &str,
        ok: bool,
        summary: &str,
        diff: Option<&openwebide_core::FileDiff>,
    ) -> Result<(), String> {
        self.backend
            .complete_tool_step(self.run.owner, self.run.session_id, id, ok, summary, diff)
            .await
    }
    fn prepare_permission(&self, id: &str) {
        self.run.gate.prepare(id);
    }
    async fn finish(&self) {
        self.run.gate.clear();
    }
}
async fn map_agent_events<B: RunBackend>(
    run: &Run,
    backend: &B,
    anchor: i64,
    events: impl Stream<Item = AgentEvent> + Send,
) {
    let mut events = Box::pin(openwebide_agent::session::events(
        SessionPersistence { run, backend },
        run.session_id,
        anchor,
        events,
    ));
    while let Some(event) = events.next().await {
        run.emit(event);
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
