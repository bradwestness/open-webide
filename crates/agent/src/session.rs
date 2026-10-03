//! Shared run planning and persisted event lifecycle. Hosts supply storage and transport primitives.
mod chat;
pub use chat::{ChatPersistence, chat_events};

use crate::AgentEvent;
use openwebide_core::{
    ChatMessage, ChatRequest, EditorContext, FileDiff, ModelRuntime, Role, RunEvent, ToolCall,
    ToolDefinition, ToolStep, TurnTelemetry,
};
use std::future::Future;

pub fn history(messages: Vec<ChatMessage>, steps: &[ToolStep]) -> Vec<ChatMessage> {
    let latest = messages.iter().rev().find_map(|message| {
        (message.role == Role::System)
            .then(|| openwebide_core::Compaction::parse(&message.content))
            .flatten()
    });
    let mut messages = if let Some(compaction) = latest {
        let session = messages.first().map_or(0, |message| message.session_id);
        let mut history = compaction.messages(session);
        let tail = messages
            .into_iter()
            .filter(|message| {
                message.id > compaction.through_message_id
                    && !(message.role == Role::System
                        && message
                            .content
                            .starts_with(openwebide_core::COMPACTION_PREFIX))
            })
            .collect();
        history.extend(openwebide_core::tool_history(tail, steps));
        history
    } else {
        openwebide_core::tool_history(
            messages
                .into_iter()
                .filter(|message| {
                    !(message.role == Role::System
                        && message
                            .content
                            .starts_with(openwebide_core::COMPACTION_PREFIX))
                })
                .collect(),
            steps,
        )
    };
    for message in &mut messages {
        if message.role == Role::Assistant {
            message.content = openwebide_core::strip_reasoning(&message.content).to_owned();
        }
    }
    messages
}
pub fn user_content(content: String, editor: Option<&EditorContext>) -> String {
    match editor {
        Some(editor) => format!("{}{content}", editor.format_prompt_injection()),
        None => content,
    }
}
pub fn tools_for_host(host_available: bool) -> Vec<ToolDefinition> {
    let mut tools = crate::vfs_tools();
    if !host_available {
        tools.retain(|tool| !crate::policy::BRIDGE_TOOLS.contains(&tool.name.as_str()));
    }
    tools
}

pub fn request(
    runtime: &ModelRuntime,
    system_prompt: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolDefinition>,
) -> ChatRequest {
    let mut request = ChatRequest {
        connection_id: runtime.connection.id,
        model: None,
        system_prompt,
        messages,
        tools,
        model_settings: Default::default(),
    };
    runtime.apply_to(&mut request);
    request
}

/// Add the same chat-only context to the request and return its persisted representation.
pub fn chat_context(
    request: &mut ChatRequest,
    environment: &openwebide_core::RunEnvironment,
) -> String {
    let content = crate::context::chat_context(environment);
    request
        .system_prompt
        .get_or_insert_with(String::new)
        .push_str(&format!("\n\n{content}"));
    format!("{}{content}", openwebide_core::RUN_CONTEXT_PREFIX)
}

/// Inputs supplied by the host; run policy and shaping are shared across every adapter.
pub struct PlanInput {
    pub environment: openwebide_core::RunEnvironment,
    pub system_prompt: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    pub content: String,
    pub editor: Option<EditorContext>,
}

pub fn plan(runtime: &ModelRuntime, input: PlanInput) -> openwebide_core::RunPlan {
    let projectless =
        input.environment.project_name.is_none() && input.environment.project_root.is_none();
    let tools = if projectless {
        projectless_tools()
    } else {
        input.tools
    };
    let request = request(runtime, input.system_prompt, input.messages, tools);
    let kind = match input.environment.project_root.as_ref() {
        Some(root) if !request.tools.is_empty() => openwebide_core::RunKind::Agent {
            project_path: root.clone(),
        },
        _ if projectless => openwebide_core::RunKind::WebChat,
        _ => openwebide_core::RunKind::Chat,
    };
    openwebide_core::RunPlan {
        connection: runtime.connection.clone(),
        transport: runtime.transport.clone(),
        request,
        environment: input.environment,
        user_content: user_content(
            input.content,
            input.editor.as_ref().filter(|_| !projectless),
        ),
        kind,
    }
}

pub fn conversation_history(entries: Vec<openwebide_core::ConversationEntry>) -> Vec<ChatMessage> {
    let (mut messages, mut steps) = (Vec::new(), Vec::new());
    for entry in entries {
        match entry {
            openwebide_core::ConversationEntry::Message(message) => messages.push(message),
            openwebide_core::ConversationEntry::ToolStep(step) => steps.push(step),
        }
    }
    history(messages, &steps)
}

pub trait RunPersistence: Send + Sync {
    fn now(&self) -> i64;
    fn message(
        &self,
        role: Role,
        content: &str,
        usage: Option<&TurnTelemetry>,
        calls: Option<&[ToolCall]>,
    ) -> impl Future<Output = Result<ChatMessage, String>> + Send;
    fn step(
        &self,
        anchor: i64,
        id: &str,
        name: &str,
        summary: &str,
        diff: Option<&FileDiff>,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn result(
        &self,
        id: &str,
        ok: bool,
        summary: &str,
        diff: Option<&FileDiff>,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn prepare_permission(&self, _id: &str) {}
    fn finish(&self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

pub struct Recorded {
    pub events: Vec<RunEvent>,
    pub terminal: bool,
}
pub struct RunRecorder<P> {
    pub persistence: P,
    session: i64,
    anchor: i64,
    reasoning: String,
    usage: Option<TurnTelemetry>,
}
impl<P: RunPersistence> RunRecorder<P> {
    pub fn new(persistence: P, session: i64, anchor: i64) -> Self {
        Self {
            persistence,
            session,
            anchor,
            reasoning: String::new(),
            usage: None,
        }
    }
    pub async fn record(&mut self, event: AgentEvent) -> Recorded {
        let mut terminal = false;
        let mut preceding = Vec::new();
        let event = match event {
            AgentEvent::Context(content) => {
                let content = format!("{}{content}", openwebide_core::RUN_CONTEXT_PREFIX);
                match self
                    .persistence
                    .message(Role::System, &content, None, None)
                    .await
                {
                    Ok(message) => RunEvent::Message { message },
                    Err(error) => {
                        terminal = true;
                        RunEvent::Error {
                            message: format!("Could not save run context: {error}"),
                        }
                    }
                }
            }
            AgentEvent::Compacted(mut compaction) => {
                compaction.through_message_id = self.anchor;
                let saved = match compaction.stored_content() {
                    Ok(content) => {
                        self.persistence
                            .message(Role::System, &content, None, None)
                            .await
                    }
                    Err(error) => Err(error.to_string()),
                };
                match saved {
                    Ok(message) => RunEvent::Message { message },
                    Err(error) => {
                        terminal = true;
                        RunEvent::Error {
                            message: format!("Could not save conversation summary: {error}"),
                        }
                    }
                }
            }
            AgentEvent::ReasoningDelta(content) => {
                self.reasoning.push_str(&content);
                RunEvent::ReasoningDelta { content }
            }
            AgentEvent::TextDelta(content) => RunEvent::Delta { content },
            AgentEvent::Telemetry(usage) => {
                self.usage = Some(usage);
                RunEvent::Telemetry { usage }
            }
            AgentEvent::TurnCalls { text, calls } => {
                let text =
                    openwebide_core::with_reasoning(&std::mem::take(&mut self.reasoning), &text);
                let usage = self.usage.take();
                let message = match self
                    .persistence
                    .message(Role::Assistant, &text, usage.as_ref(), Some(&calls))
                    .await
                {
                    Ok(message) => {
                        self.anchor = message.id;
                        message
                    }
                    Err(_) => ChatMessage {
                        id: 0,
                        session_id: self.session,
                        role: Role::Assistant,
                        content: text,
                        created_at: self.persistence.now(),
                        tool_calls: Some(calls),
                        tool_call_id: None,
                        usage,
                    },
                };
                RunEvent::Interim { message }
            }
            AgentEvent::ToolCall { id, name, summary } => {
                self.usage = None;
                let _ = self
                    .persistence
                    .step(self.anchor, &id, &name, &summary, None)
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
                self.usage = None;
                self.persistence.prepare_permission(&id);
                let _ = self
                    .persistence
                    .step(self.anchor, &id, &name, &summary, diff.as_ref())
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
                let saved = self
                    .persistence
                    .result(&id, ok, &summary, diff.as_ref())
                    .await;
                let result = RunEvent::ToolResult {
                    id,
                    name,
                    ok,
                    summary,
                    diff,
                };
                match saved {
                    Ok(()) => result,
                    Err(error) => {
                        preceding.push(result);
                        terminal = true;
                        RunEvent::Error {
                            message: format!("failed to save tool result: {error}"),
                        }
                    }
                }
            }
            AgentEvent::FinalText(text) => {
                terminal = true;
                let text = openwebide_core::with_reasoning(&self.reasoning, &text);
                match self
                    .persistence
                    .message(Role::Assistant, &text, self.usage.take().as_ref(), None)
                    .await
                {
                    Ok(message) => RunEvent::Done { message },
                    Err(error) => RunEvent::Error {
                        message: format!("failed to save reply: {error}"),
                    },
                }
            }
            AgentEvent::Cancelled => {
                terminal = true;
                RunEvent::Cancelled
            }
            AgentEvent::Error(message) => {
                terminal = true;
                RunEvent::Error { message }
            }
        };
        preceding.push(event);
        Recorded {
            events: preceding,
            terminal,
        }
    }
}

/// Compact and durably record the replacement before any completion stream starts.
#[allow(
    clippy::too_many_arguments,
    reason = "The shared preparation uses model, cancellation and persistence primitives"
)]
pub async fn compact_request<P, S, C, D>(
    provider: &P,
    source: &S,
    request: &mut ChatRequest,
    cancel: &C,
    persistence: D,
    session: i64,
    anchor: i64,
) -> Recorded
where
    P: openwebide_llm::LlmProvider,
    S: crate::compaction::CompactionSource,
    C: crate::CancelCheck + Sync,
    D: RunPersistence,
{
    if cancel.check().await {
        return Recorded {
            events: vec![RunEvent::Cancelled],
            terminal: true,
        };
    }
    let result = {
        let prepare = Box::pin(crate::compaction::prepare(provider, source, request));
        let cancelled = Box::pin(async { cancel.cancelled().await });
        match futures::future::select(prepare, cancelled).await {
            futures::future::Either::Left((result, _)) => result,
            futures::future::Either::Right(_) => {
                return Recorded {
                    events: vec![RunEvent::Cancelled],
                    terminal: true,
                };
            }
        }
    };
    if cancel.check().await {
        return Recorded {
            events: vec![RunEvent::Cancelled],
            terminal: true,
        };
    }
    match result {
        Ok(Some(compaction)) => {
            RunRecorder::new(persistence, session, anchor)
                .record(AgentEvent::Compacted(compaction))
                .await
        }
        Ok(None) => Recorded {
            events: vec![],
            terminal: false,
        },
        Err(message) => Recorded {
            events: vec![RunEvent::Error { message }],
            terminal: true,
        },
    }
}

/// Shared streaming driver. Stops/drops the agent before publishing a terminal event.
pub fn events<'a, P: RunPersistence + 'a>(
    persistence: P,
    session: i64,
    anchor: i64,
    input: impl futures::Stream<Item = AgentEvent> + Send + 'a,
) -> impl futures::Stream<Item = RunEvent> + Send + 'a {
    use futures::StreamExt;
    let input: std::pin::Pin<Box<dyn futures::Stream<Item = AgentEvent> + Send + 'a>> =
        Box::pin(input);
    futures::stream::unfold(
        (
            RunRecorder::new(persistence, session, anchor),
            Some(input),
            std::collections::VecDeque::new(),
        ),
        |(mut recorder, mut input, mut pending)| async move {
            loop {
                if let Some(event) = pending.pop_front() {
                    return Some((event, (recorder, input, pending)));
                }
                let next = match input.as_mut() {
                    Some(input) => input.next().await,
                    None => return None,
                };
                let Some(next) = next else {
                    recorder.persistence.finish().await;
                    return None;
                };
                let recorded = recorder.record(next).await;
                if recorded.terminal {
                    input.take();
                    recorder.persistence.finish().await;
                }
                pending.extend(recorded.events);
            }
        },
    )
}

/// Read-only host information and web tools available without opening a workspace.
pub fn projectless_tools() -> Vec<ToolDefinition> {
    crate::vfs_tools()
        .into_iter()
        .filter(|tool| is_projectless_tool(&tool.name))
        .collect()
}

pub fn is_projectless_tool(name: &str) -> bool {
    matches!(name, "search_web" | "fetch_web_page" | "host_info")
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    struct Persistence(Arc<AtomicUsize>);
    impl RunPersistence for Persistence {
        fn now(&self) -> i64 {
            0
        }
        async fn message(
            &self,
            _role: Role,
            _content: &str,
            _usage: Option<&TurnTelemetry>,
            _calls: Option<&[ToolCall]>,
        ) -> Result<ChatMessage, String> {
            Err("offline".into())
        }
        async fn step(
            &self,
            _anchor: i64,
            _id: &str,
            _name: &str,
            _summary: &str,
            _diff: Option<&FileDiff>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn result(
            &self,
            _id: &str,
            _ok: bool,
            _summary: &str,
            _diff: Option<&FileDiff>,
        ) -> Result<(), String> {
            Err("offline".into())
        }
        async fn finish(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    struct DropGuard(Arc<AtomicBool>);
    impl Drop for DropGuard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn summary_history_retains_original_task_and_replays_only_new_messages() {
        let message = |id, role, content: &str| ChatMessage {
            id,
            session_id: 1,
            role,
            content: content.into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        };
        let old = message(1, Role::User, "old task");
        let current = message(3, Role::User, "current exact task");
        let compaction = openwebide_core::Compaction {
            summary: "Old work completed".into(),
            retained: vec![current.clone()],
            through_message_id: 3,
        };
        let stored = message(4, Role::System, &compaction.stored_content().unwrap());
        let originals = vec![
            old.clone(),
            message(2, Role::Assistant, "old answer"),
            current.clone(),
            stored,
            message(5, Role::Assistant, "continued answer"),
            message(6, Role::User, "next task"),
        ];
        let history = history(originals.clone(), &[]);
        assert_eq!(originals[0], old);
        assert_eq!(history.len(), 4);
        assert!(history[0].content.contains("Old work completed"));
        assert_eq!(history[1], current);
        assert_eq!(history[2].id, 5);
        assert_eq!(history[3].id, 6);
        let broken = vec![
            old.clone(),
            message(2, Role::System, "[conversation compaction]\ninvalid"),
        ];
        assert_eq!(super::history(broken, &[]), vec![old]);
    }

    #[test]
    fn failed_summary_save_stops_before_the_next_model_request() {
        futures::executor::block_on(async {
            let finished = Arc::new(AtomicUsize::new(0));
            let compaction = openwebide_core::Compaction {
                summary: "summary".into(),
                retained: vec![],
                through_message_id: 0,
            };
            let output: Vec<_> = events(
                Persistence(finished.clone()),
                1,
                3,
                futures::stream::iter([
                    AgentEvent::Compacted(compaction),
                    AgentEvent::TextDelta("must not continue".into()),
                ]),
            )
            .collect()
            .await;
            assert!(
                matches!(output.as_slice(), [RunEvent::Error { message }] if message.contains("save conversation summary"))
            );
            assert_eq!(finished.load(Ordering::Relaxed), 1);
        });
    }

    #[test]
    fn fatal_save_failure_stops_source_and_preserves_actual_tool_result() {
        futures::executor::block_on(async {
            let finished = Arc::new(AtomicUsize::new(0));
            let dropped = Arc::new(AtomicBool::new(false));
            let polled = Arc::new(AtomicUsize::new(0));
            let observed = polled.clone();
            let guard = DropGuard(dropped.clone());
            let mut source = [
                AgentEvent::ToolResult {
                    id: "write".into(),
                    name: "write_file".into(),
                    ok: true,
                    summary: "written".into(),
                    diff: None,
                },
                AgentEvent::TextDelta("must not continue".into()),
            ]
            .into_iter();
            let input = futures::stream::poll_fn(move |_| {
                let _ = &guard;
                observed.fetch_add(1, Ordering::Relaxed);
                std::task::Poll::Ready(source.next())
            });
            let mut output = Box::pin(events(Persistence(finished.clone()), 1, 2, input));
            assert!(matches!(
                output.next().await,
                Some(RunEvent::ToolResult { ok: true, .. })
            ));
            assert!(dropped.load(Ordering::Relaxed));
            assert_eq!(finished.load(Ordering::Relaxed), 1);
            assert!(matches!(output.next().await, Some(RunEvent::Error { .. })));
            assert!(output.next().await.is_none());
            assert_eq!(polled.load(Ordering::Relaxed), 1);
        });
    }
}
