//! Shared run planning and persisted event lifecycle. Hosts supply storage and transport primitives.
use crate::AgentEvent;
use openwebide_core::{
    ChatMessage, ChatRequest, EditorContext, FileDiff, ModelRuntime, Role, RunEvent, ToolCall,
    ToolDefinition, ToolStep, TurnTelemetry,
};
use std::future::Future;

pub fn history(messages: Vec<ChatMessage>, steps: &[ToolStep]) -> Vec<ChatMessage> {
    let mut messages = openwebide_core::tool_history(messages, steps);
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
    ChatRequest {
        connection_id: runtime.connection.id,
        model: runtime.connection.model.clone(),
        system_prompt,
        messages,
        tools: if runtime.settings.tools == Some(false) {
            Vec::new()
        } else {
            tools
        },
        model_settings: runtime.settings.clone(),
    }
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
