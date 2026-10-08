//! Parallel child scheduling and safe fast-model fallback, independent of host runtime.
use super::{TaskRun, budget::TaskBudget, child_request};
use crate::{AgentEvent, CancelCheck, ToolOutcome, model::ModelSource};
use futures::{Stream, StreamExt, stream};
use openwebide_core::{
    ChatRequest, NewAgentTask, Role, RunEvent, TaskEvent, TaskRequest, TaskStatus, TaskUpdate,
    tasks::{MAX_TASK_DEPTH, task_run_id},
};
use std::{
    collections::{BTreeMap, VecDeque},
    pin::Pin,
};

pub type ChildStream = Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;
/// Hosts construct the shared scoped agent loop with inherited gates and fresh executors.
/// Return an unpolled stream: a checkpoint must be acknowledged before its tool runs.
pub trait TaskHost: ModelSource + CancelCheck + Clone + Send + Sync + 'static {
    fn now_ms(&self) -> u64;
    fn run_child(
        &self,
        request: ChatRequest,
        scope: String,
        depth: usize,
        budget: TaskBudget,
    ) -> impl std::future::Future<Output = Result<ChildStream, String>> + Send;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskExecutionEvent {
    Update(Box<TaskUpdate>),
    Finished(ToolOutcome),
}
pub type TaskExecutionStream = Pin<Box<dyn Stream<Item = TaskExecutionEvent> + Send>>;

struct Child<H> {
    host: H,
    parent: ChatRequest,
    task: NewAgentTask,
    depth: usize,
    budget: TaskBudget,
    run: TaskRun,
    started: bool,
    prepared: bool,
    candidates: VecDeque<ChatRequest>,
    source: Option<ChildStream>,
    pending: VecDeque<TaskUpdate>,
}
async fn candidates<H: TaskHost>(
    host: &H,
    parent: &ChatRequest,
    task: &NewAgentTask,
    depth: usize,
) -> Result<VecDeque<ChatRequest>, String> {
    let mut primary = child_request(parent, task, depth)?;
    primary.model_settings.max_output_tokens = Some(
        primary
            .model_settings
            .max_output_tokens
            .unwrap_or(2048)
            .min(2048),
    );
    let mut requests = VecDeque::new();
    if let Some(fast) = &parent.model_settings.fast
        && (fast.server_id != parent.connection_id || Some(&fast.model) != parent.model.as_ref())
        && let Ok(runtime) = host.runtime(fast).await
        && (primary.tools.is_empty() || runtime.settings.tools != Some(false))
    {
        let mut request = primary.clone();
        request.connection_id = fast.server_id;
        runtime.apply_to(&mut request);
        request.model_settings.max_output_tokens = Some(
            request
                .model_settings
                .max_output_tokens
                .unwrap_or(2048)
                .min(2048)
                .min(
                    request
                        .model_settings
                        .context_limit
                        .map_or(2048, |limit| (limit / 4).max(1)),
                ),
        );
        if primary.tools.is_empty() || !request.tools.is_empty() {
            requests.push_back(request);
        }
    }
    requests.push_back(primary);
    Ok(requests)
}
fn child<H: TaskHost>(
    host: H,
    parent: ChatRequest,
    task: NewAgentTask,
    parent_id: String,
    index: usize,
    depth: usize,
    budget: TaskBudget,
) -> Pin<Box<dyn Stream<Item = TaskUpdate> + Send>> {
    let id = task_run_id(&parent_id, index);
    let now = host.now_ms();
    Box::pin(stream::unfold(
        Child {
            host,
            parent,
            run: TaskRun::new(id, parent_id, &task, now),
            task,
            depth,
            budget,
            started: false,
            prepared: false,
            candidates: VecDeque::new(),
            source: None,
            pending: VecDeque::new(),
        },
        |mut state| async move {
            loop {
                if let Some(update) = state.pending.pop_front() {
                    return Some((update, state));
                }
                if state.run.snapshot.task.status.finished() {
                    return None;
                }
                if !state.started {
                    state.started = true;
                    let update = state.run.started(&state.task, state.host.now_ms());
                    return Some((update, state));
                }
                if !state.prepared {
                    state.prepared = true;
                    let result = {
                        let prepare = Box::pin(candidates(
                            &state.host,
                            &state.parent,
                            &state.task,
                            state.depth,
                        ));
                        let cancel = Box::pin(state.host.cancelled());
                        match futures::future::select(prepare, cancel).await {
                            futures::future::Either::Left((result, _)) => Some(result),
                            futures::future::Either::Right(_) => None,
                        }
                    };
                    match result {
                        Some(Ok(requests)) => state.candidates = requests,
                        Some(Err(error)) => state.pending.extend(
                            state
                                .run
                                .observe(AgentEvent::Error(error), state.host.now_ms()),
                        ),
                        None => state.pending.extend(
                            state
                                .run
                                .observe(AgentEvent::Cancelled, state.host.now_ms()),
                        ),
                    }
                    continue;
                }
                if state.source.is_none() {
                    if state.host.check().await {
                        state.pending.extend(
                            state
                                .run
                                .observe(AgentEvent::Cancelled, state.host.now_ms()),
                        );
                        continue;
                    }
                    let Some(request) = state.candidates.pop_front() else {
                        state.pending.extend(state.run.observe(
                            AgentEvent::Error("No child model was available".into()),
                            state.host.now_ms(),
                        ));
                        continue;
                    };
                    let result = {
                        let launch = Box::pin(state.host.run_child(
                            request,
                            state.run.snapshot.task.id.clone(),
                            state.depth,
                            state.budget.clone(),
                        ));
                        let cancel = Box::pin(state.host.cancelled());
                        match futures::future::select(launch, cancel).await {
                            futures::future::Either::Left((result, _)) => Some(result),
                            futures::future::Either::Right(_) => None,
                        }
                    };
                    match result {
                        Some(Ok(source)) => state.source = Some(source),
                        Some(Err(error)) if !state.candidates.is_empty() => state
                            .pending
                            .push_back(fallback_note(&mut state.run, &error, state.host.now_ms())),
                        Some(Err(error)) => state.pending.extend(
                            state
                                .run
                                .observe(AgentEvent::Error(error), state.host.now_ms()),
                        ),
                        None => state.pending.extend(
                            state
                                .run
                                .observe(AgentEvent::Cancelled, state.host.now_ms()),
                        ),
                    }
                    continue;
                }
                // The child loop owns cancellation so atomic file writes can finish safely.
                let event = state
                    .source
                    .as_mut()
                    .expect("Child source started")
                    .next()
                    .await
                    .unwrap_or_else(|| {
                        AgentEvent::Error("Child run ended without completion".into())
                    });
                if let AgentEvent::Error(error) = &event
                    && state.run.snapshot.task.tool_count == 0
                    && !state.candidates.is_empty()
                {
                    state.source = None;
                    state.run.finish_turn();
                    state.run.snapshot.run.text.clear();
                    state.run.snapshot.run.reasoning.clear();
                    state.pending.push_back(fallback_note(
                        &mut state.run,
                        error,
                        state.host.now_ms(),
                    ));
                    continue;
                }
                state
                    .pending
                    .extend(state.run.observe(event, state.host.now_ms()));
            }
        },
    ))
}
fn fallback_note(run: &mut TaskRun, error: &str, now_ms: u64) -> TaskUpdate {
    let content = format!(
        "Fast model failed before tool execution; retrying with the primary model. {}",
        error.chars().take(1024).collect::<String>()
    );
    run.emit(TaskEvent::Run {
        event: Box::new(RunEvent::Message {
            message: TaskRun::message(Role::System, content, now_ms, None),
        }),
    })
}

/// One task call multiplexes child progress; failures preserve successful siblings.
pub fn run_tasks<H: TaskHost>(
    host: H,
    parent: ChatRequest,
    parent_id: String,
    request: TaskRequest,
    parent_depth: usize,
    budget: TaskBudget,
) -> Result<TaskExecutionStream, String> {
    request.validate()?;
    let depth = parent_depth
        .checked_add(1)
        .filter(|depth| *depth <= MAX_TASK_DEPTH)
        .ok_or_else(|| {
            format!("Child agents support at most {MAX_TASK_DEPTH} levels of delegation.")
        })?;
    let count = request.tasks.len();
    budget.reserve(count)?;
    let children =
        stream::select_all(request.tasks.into_iter().enumerate().map(|(index, task)| {
            child(
                host.clone(),
                parent.clone(),
                task,
                parent_id.clone(),
                index,
                depth,
                budget.clone(),
            )
        }));
    Ok(Box::pin(stream::unfold(
        (children, BTreeMap::new(), false),
        move |(mut children, mut completed, finished)| async move {
            if finished {
                return None;
            }
            if let Some(update) = children.next().await {
                if update.task.status.finished() {
                    completed.insert(update.task.id.clone(), update.task.clone());
                }
                return Some((
                    TaskExecutionEvent::Update(Box::new(update)),
                    (children, completed, false),
                ));
            }
            let successes = completed
                .values()
                .filter(|task| task.status == TaskStatus::Completed)
                .count();
            let content = completed
                .values()
                .map(|task| {
                    let result = task.result.as_deref().unwrap_or("No result recorded");
                    let clipped = &result[..result.floor_char_boundary(result.len().min(8192))];
                    format!(
                        "{} ({:?})\n{}{}",
                        task.description,
                        task.status,
                        clipped,
                        if clipped.len() < result.len() {
                            "\n[Result truncated]"
                        } else {
                            ""
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            Some((
                TaskExecutionEvent::Finished(ToolOutcome {
                    ok: successes > 0,
                    content,
                    summary: format!("{successes}/{count} child tasks completed"),
                    diff: None,
                }),
                (children, completed, true),
            ))
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{FutureExt, channel::oneshot};
    use openwebide_core::{ChatMessage, ModelRuntime, ModelSelection};
    use std::sync::{Arc, Mutex};
    type Recorded = Arc<Mutex<Vec<(String, usize, ChatRequest)>>>;
    #[derive(Clone, Default)]
    struct Host {
        requests: Recorded,
        replies: Arc<Mutex<BTreeMap<String, oneshot::Receiver<AgentEvent>>>>,
        fast_effect: bool,
        tool_selection: openwebide_core::ToolSelection,
    }
    impl ModelSource for Host {
        async fn runtime(&self, selection: &ModelSelection) -> Result<ModelRuntime, String> {
            Ok(ModelRuntime {
                connection: openwebide_core::Connection {
                    id: selection.server_id,
                    name: "Fast".into(),
                    kind: openwebide_core::ProviderKind::Ollama,
                    base_url: "http://localhost:11434".into(),
                    model: Some(selection.model.clone()),
                    enabled: true,
                    context_limit: Some(8192),
                    tool_stream_unsupported: false,
                    tool_stream_revision: 0,
                    tool_selection: self.tool_selection.clone(),
                },
                settings: openwebide_core::ModelSettings {
                    context_limit: Some(32768),
                    max_output_tokens: Some(8192),
                    ..Default::default()
                },
                transport: Default::default(),
            })
        }
    }
    impl CancelCheck for Host {
        async fn check(&self) -> bool {
            false
        }
        async fn cancelled(&self) {
            futures::future::pending::<()>().await;
        }
    }
    impl TaskHost for Host {
        fn now_ms(&self) -> u64 {
            1000
        }
        async fn run_child(
            &self,
            request: ChatRequest,
            scope: String,
            depth: usize,
            _: TaskBudget,
        ) -> Result<ChildStream, String> {
            self.requests
                .lock()
                .unwrap()
                .push((scope.clone(), depth, request.clone()));
            if request.model.as_deref() == Some("fast") {
                let mut events = Vec::new();
                if self.fast_effect {
                    events.push(AgentEvent::ToolCall {
                        id: format!("{scope}.a1t1c0"),
                        name: "write_file".into(),
                        summary: "Write file".into(),
                    });
                }
                events.push(AgentEvent::Error("Fast model unavailable".into()));
                return Ok(Box::pin(stream::iter(events)));
            }
            let receiver = self
                .replies
                .lock()
                .unwrap()
                .remove(&request.messages[0].content)
                .unwrap();
            Ok(Box::pin(stream::once(
                async move { receiver.await.unwrap() },
            )))
        }
    }
    fn parent() -> ChatRequest {
        ChatRequest {
            connection_id: 1,
            model: Some("primary".into()),
            model_settings: Default::default(),
            system_prompt: Some("Trusted instructions".into()),
            tools: Vec::new(),
            messages: vec![ChatMessage {
                id: 1,
                session_id: 1,
                role: Role::User,
                content: "Private parent context".into(),
                created_at: 1,
                tool_calls: None,
                tool_call_id: None,
                usage: None,
            }],
        }
    }
    fn request(prompts: &[&str]) -> TaskRequest {
        TaskRequest {
            tasks: prompts
                .iter()
                .map(|prompt| NewAgentTask {
                    description: format!("Task {prompt}"),
                    prompt: (*prompt).into(),
                })
                .collect(),
        }
    }
    #[test]
    fn fast_model_selection_cannot_widen_tools_and_chat_only_falls_back() {
        futures::executor::block_on(async {
            let mut parent = parent();
            parent.tools = crate::vfs_tools();
            parent.model_settings.fast = Some(ModelSelection {
                server_id: 2,
                model: "fast".into(),
            });
            let task = NewAgentTask {
                description: "child".into(),
                prompt: "go".into(),
            };
            for selection in [
                openwebide_core::ToolSelection::Selected(vec!["read_file".into()]),
                openwebide_core::ToolSelection::ChatOnly,
            ] {
                let host = Host {
                    tool_selection: selection.clone(),
                    ..Default::default()
                };
                let candidates = candidates(&host, &parent, &task, 1).await.unwrap();
                if selection == openwebide_core::ToolSelection::ChatOnly {
                    assert_eq!(candidates.len(), 1);
                    assert_eq!(candidates[0].connection_id, parent.connection_id);
                } else {
                    assert_eq!(candidates.len(), 2);
                    assert_eq!(
                        candidates[0]
                            .tools
                            .iter()
                            .map(|tool| tool.name.as_str())
                            .collect::<Vec<_>>(),
                        vec!["read_file"]
                    );
                }
            }
        });
    }

    #[test]
    fn blocked_child_does_not_block_siblings_and_failure_preserves_results() {
        futures::executor::block_on(async {
            let host = Host::default();
            let (send_a, receive_a) = oneshot::channel();
            let (send_b, receive_b) = oneshot::channel();
            host.replies
                .lock()
                .unwrap()
                .extend([("A".into(), receive_a), ("B".into(), receive_b)]);
            let mut events = run_tasks(
                host.clone(),
                parent(),
                "a1t1c0".into(),
                request(&["A", "B"]),
                0,
                TaskBudget::default(),
            )
            .unwrap();
            assert!(matches!(
                events.next().await,
                Some(TaskExecutionEvent::Update(_))
            ));
            assert!(matches!(
                events.next().await,
                Some(TaskExecutionEvent::Update(_))
            ));
            assert!(events.next().now_or_never().is_none());
            assert_eq!(host.requests.lock().unwrap().len(), 2);
            send_b
                .send(AgentEvent::FinalText("Sibling findings".into()))
                .unwrap();
            let update = events.next().await.unwrap();
            assert!(
                matches!(update, TaskExecutionEvent::Update(update) if update.task.id.ends_with("task2") && update.task.status == TaskStatus::Completed)
            );
            send_a
                .send(AgentEvent::Error("Child failed".into()))
                .unwrap();
            let mut result = None;
            while let Some(event) = events.next().await {
                if let TaskExecutionEvent::Finished(outcome) = event {
                    result = Some(outcome);
                }
            }
            let result = result.unwrap();
            assert!(result.ok);
            assert!(
                result.content.contains("Sibling findings")
                    && result.content.contains("Child failed")
            );
            assert_eq!(result.summary, "1/2 child tasks completed");
            let requests = host.requests.lock().unwrap();
            assert_ne!(requests[0].0, requests[1].0);
            for (_, depth, request) in requests.iter() {
                assert_eq!(*depth, 1);
                assert_eq!(request.messages.len(), 1);
                assert!(
                    !request.messages[0]
                        .content
                        .contains("Private parent context")
                );
            }
        });
    }
    #[test]
    fn fast_failure_falls_back_only_before_tool_effects() {
        futures::executor::block_on(async {
            for fast_effect in [false, true] {
                let host = Host {
                    fast_effect,
                    ..Default::default()
                };
                let (send, receive) = oneshot::channel();
                host.replies.lock().unwrap().insert("A".into(), receive);
                send.send(AgentEvent::FinalText("Primary result".into()))
                    .unwrap();
                let mut parent = parent();
                parent.model_settings.fast = Some(ModelSelection {
                    server_id: 2,
                    model: "fast".into(),
                });
                let events: Vec<_> = run_tasks(
                    host.clone(),
                    parent,
                    "a1t1c0".into(),
                    request(&["A"]),
                    0,
                    TaskBudget::default(),
                )
                .unwrap()
                .collect()
                .await;
                let result = events
                    .iter()
                    .find_map(|event| match event {
                        TaskExecutionEvent::Finished(outcome) => Some(outcome),
                        TaskExecutionEvent::Update(_) => None,
                    })
                    .unwrap();
                assert_eq!(result.ok, !fast_effect);
                assert!(host.requests.lock().unwrap().iter().all(|(_, _, request)| {
                    request.model_settings.max_output_tokens == Some(2048)
                }));
                assert_eq!(
                    host.requests.lock().unwrap().len(),
                    if fast_effect { 1 } else { 2 }
                );
            }
        });
    }
}
