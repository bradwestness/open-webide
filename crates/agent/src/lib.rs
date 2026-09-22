//! The tool-calling agent loop.
//!
//! Drives a [`LlmProvider`] through the model → tool call → execute → result →
//! model cycle, emitting an [`AgentEvent`] for each step so the UI can render
//! the agent's work as it happens. The loop is pure: it owns no persistence and
//! no filesystem — the host supplies a [`ToolExecutor`] and decides what to do
//! with the final text.

use std::future::Future;
use std::pin::Pin;

use futures::{Stream, stream};
use openwebide_core::{
    ChatMessage, ChatRequest, ChatResponse, FileDiff, Role, ToolCall, ToolDefinition,
};
use openwebide_llm::LlmProvider;

/// Budgets that bound a single agent run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentConfig {
    /// Maximum model round-trips. Each turn is one `chat_tools` call; a turn
    /// that returns tool calls still counts as one turn even if it runs many
    /// tools.
    pub max_turns: usize,
    /// Maximum total tool executions across the whole run.
    pub max_tool_calls: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: 12,
            max_tool_calls: 24,
        }
    }
}

/// The result of executing one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    /// Whether the tool ran without error.
    pub ok: bool,
    /// The result fed back to the model as a `tool` message.
    pub content: String,
    /// A short human-readable summary for the UI.
    pub summary: String,
    /// A diff, when the tool edited a file.
    pub diff: Option<FileDiff>,
}

/// A step in an agent run, emitted to the UI as it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// The model requested a tool call.
    ToolCall {
        id: String,
        name: String,
        /// What the call is about to do (e.g. `read src/main.rs`).
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
    /// The model's final text answer.
    FinalText(String),
    /// The run stopped with an error (budget exhausted or provider failure).
    Error(String),
    /// The user cancelled the run; no final text will follow.
    Cancelled,
}

/// Executes the tool calls the model requests.
///
/// Implementations are responsible for confining their work (e.g. to a
/// workspace) and for reporting a useful [`ToolOutcome`].
pub trait ToolExecutor: Send {
    /// A short human-readable description of what this call will do, shown in
    /// the UI before the tool runs.
    fn describe(&self, call: &ToolCall) -> String;
    /// Run the call and report the outcome.
    fn execute(&self, call: &ToolCall) -> impl Future<Output = ToolOutcome> + Send;
}

/// Decides whether the current run should stop.
///
/// The loop calls this at step boundaries — before each model call and before
/// each tool execution — so a cancel takes effect at the next boundary
/// without interrupting an in-flight request.
pub trait CancelCheck: Send {
    /// Returns `true` when the run should stop.
    ///
    /// `async fn` in a trait cannot express the `Send` bound the agent loop
    /// needs (the check's future is awaited inside a `Send` stream), so this
    /// is an explicit `impl Future` (and impls allow the lint that suggests
    /// the `async fn` form).
    fn check(&self) -> impl Future<Output = bool> + Send;
}

/// A cancel check that never fires; for hosts without cancellation.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopCancel;

impl CancelCheck for NoopCancel {
    #[allow(clippy::manual_async_fn)]
    fn check(&self) -> impl Future<Output = bool> + Send {
        async { false }
    }
}

/// Run the agent loop, streaming one [`AgentEvent`] per step.
///
/// The loop ends with a [`AgentEvent::FinalText`] (the model's answer), an
/// [`AgentEvent::Error`] (a budget was exhausted or the provider failed), or
/// an [`AgentEvent::Cancelled`] (the cancel check fired at a step boundary).
pub fn run<P, T, C>(
    provider: P,
    executor: T,
    request: ChatRequest,
    config: AgentConfig,
    cancel: C,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send + 'static>>
where
    P: LlmProvider + 'static,
    T: ToolExecutor + 'static,
    C: CancelCheck + 'static,
{
    Box::pin(stream::unfold(
        LoopState {
            provider,
            executor,
            cancel,
            connection_id: request.connection_id,
            system_prompt: request.system_prompt,
            model: request.model,
            messages: request.messages,
            tools: request.tools,
            config,
            turn: 0,
            tool_calls: 0,
            pending: Vec::new(),
            current: None,
            next: Next::CallModel,
        },
        |mut state| async move {
            loop {
                match state.next {
                    Next::Stop => return None,
                    Next::CallModel => {
                        if state.cancel.check().await {
                            state.next = Next::Stop;
                            return Some((AgentEvent::Cancelled, state));
                        }
                        if state.turn >= state.config.max_turns {
                            state.next = Next::Stop;
                            return Some((
                                AgentEvent::Error(format!(
                                    "turn budget exhausted after {} turns",
                                    state.turn
                                )),
                                state,
                            ));
                        }
                        state.turn += 1;
                        let req = ChatRequest {
                            connection_id: state.connection_id,
                            system_prompt: state.system_prompt.clone(),
                            model: state.model.clone(),
                            messages: state.messages.clone(),
                            tools: state.tools.clone(),
                        };
                        match state.provider.chat_tools(&req).await {
                            Err(e) => {
                                state.next = Next::Stop;
                                return Some((AgentEvent::Error(e.to_string()), state));
                            }
                            Ok(ChatResponse::Text(text)) => {
                                state.next = Next::Stop;
                                return Some((AgentEvent::FinalText(text), state));
                            }
                            Ok(ChatResponse::ToolCalls(calls)) => {
                                state.messages.push(ChatMessage {
                                    id: 0,
                                    session_id: 0,
                                    role: Role::Assistant,
                                    content: String::new(),
                                    created_at: 0,
                                    tool_calls: Some(calls.clone()),
                                    tool_call_id: None,
                                });
                                state.pending = calls;
                                state.next = Next::EmitToolCall;
                            }
                        }
                    }
                    Next::EmitToolCall => {
                        if state.pending.is_empty() {
                            state.next = Next::CallModel;
                            continue;
                        }
                        if state.tool_calls >= state.config.max_tool_calls {
                            state.next = Next::Stop;
                            return Some((
                                AgentEvent::Error(format!(
                                    "tool budget exhausted after {} tool calls",
                                    state.tool_calls
                                )),
                                state,
                            ));
                        }
                        let call = state.pending.remove(0);
                        let summary = state.executor.describe(&call);
                        state.current = Some(call.clone());
                        state.next = Next::RunTool;
                        return Some((
                            AgentEvent::ToolCall {
                                id: call.id,
                                name: call.name,
                                summary,
                            },
                            state,
                        ));
                    }
                    Next::RunTool => {
                        if state.cancel.check().await {
                            state.next = Next::Stop;
                            return Some((AgentEvent::Cancelled, state));
                        }
                        let call = state
                            .current
                            .take()
                            .expect("RunTool without a current call");
                        state.tool_calls += 1;
                        let outcome = state.executor.execute(&call).await;
                        state.messages.push(ChatMessage {
                            id: 0,
                            session_id: 0,
                            role: Role::Tool,
                            content: outcome.content.clone(),
                            created_at: 0,
                            tool_calls: None,
                            tool_call_id: Some(call.id.clone()),
                        });
                        state.next = Next::EmitToolCall;
                        return Some((
                            AgentEvent::ToolResult {
                                id: call.id,
                                name: call.name,
                                ok: outcome.ok,
                                summary: outcome.summary,
                                diff: outcome.diff,
                            },
                            state,
                        ));
                    }
                }
            }
        },
    ))
}

/// The agent loop's mutable state, threaded through `stream::unfold`.
struct LoopState<P, T, C> {
    provider: P,
    executor: T,
    cancel: C,
    connection_id: i64,
    system_prompt: Option<String>,
    model: Option<String>,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolDefinition>,
    config: AgentConfig,
    turn: usize,
    tool_calls: usize,
    /// Tool calls from the most recent model response, not yet executed.
    pending: Vec<ToolCall>,
    /// The call currently being executed (set between Emit and Run).
    current: Option<ToolCall>,
    next: Next,
}

/// Which step the loop takes next.
#[derive(Debug, Clone, Copy)]
enum Next {
    CallModel,
    EmitToolCall,
    RunTool,
    Stop,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use futures::StreamExt;
    use openwebide_core::{ModelInfo, ProviderKind};
    use openwebide_llm::ProviderError;

    /// A scriptable provider: `chat_tools` pops the next queued response and
    /// records every request it is given.
    struct FakeProvider {
        responses: Arc<Mutex<VecDeque<Result<ChatResponse, ProviderError>>>>,
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }

    impl FakeProvider {
        fn new(
            responses: Vec<Result<ChatResponse, ProviderError>>,
        ) -> (Self, Arc<Mutex<Vec<ChatRequest>>>) {
            let requests = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    responses: Arc::new(Mutex::new(responses.into())),
                    requests: requests.clone(),
                },
                requests,
            )
        }
    }

    impl LlmProvider for FakeProvider {
        fn kind(&self) -> ProviderKind {
            ProviderKind::Ollama
        }
        async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
            Ok(Vec::new())
        }
        async fn chat(&self, _request: &ChatRequest) -> Result<String, ProviderError> {
            Ok(String::new())
        }
        fn chat_stream(
            &self,
            _request: &ChatRequest,
        ) -> Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send + 'static>> {
            Box::pin(stream::empty())
        }
        async fn chat_tools(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.requests.lock().unwrap().push(request.clone());
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(ChatResponse::Text(String::new())))
        }
    }

    /// An executor that pops queued outcomes and describes calls by name + args.
    struct FakeExecutor {
        outcomes: Arc<Mutex<VecDeque<ToolOutcome>>>,
    }

    impl FakeExecutor {
        fn new(outcomes: Vec<ToolOutcome>) -> Self {
            Self {
                outcomes: Arc::new(Mutex::new(outcomes.into())),
            }
        }
    }

    impl ToolExecutor for FakeExecutor {
        fn describe(&self, call: &ToolCall) -> String {
            format!("{} {}", call.name, call.arguments)
        }
        async fn execute(&self, _call: &ToolCall) -> ToolOutcome {
            self.outcomes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(ToolOutcome {
                    ok: true,
                    content: String::new(),
                    summary: String::new(),
                    diff: None,
                })
        }
    }

    fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    fn outcome(content: &str, summary: &str) -> ToolOutcome {
        ToolOutcome {
            ok: true,
            content: content.into(),
            summary: summary.into(),
            diff: None,
        }
    }

    fn request() -> ChatRequest {
        ChatRequest {
            connection_id: 1,
            system_prompt: Some("be brief".into()),
            model: Some("test-model".into()),
            messages: vec![ChatMessage {
                id: 0,
                session_id: 1,
                role: Role::User,
                content: "fix the failing test".into(),
                created_at: 0,
                tool_calls: None,
                tool_call_id: None,
            }],
            tools: vec![ToolDefinition {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: serde_json::json!({ "type": "object" }),
            }],
        }
    }

    fn collect(events: impl Stream<Item = AgentEvent>) -> Vec<AgentEvent> {
        futures::executor::block_on(async { events.collect::<Vec<_>>().await })
    }

    #[test]
    fn runs_tools_then_final_text() {
        let read = call("call_0", "read_file", r#"{"path":"src/main.rs"}"#);
        let (provider, requests) = FakeProvider::new(vec![
            Ok(ChatResponse::ToolCalls(vec![read.clone()])),
            Ok(ChatResponse::Text("fixed it".into())),
        ]);
        let executor = FakeExecutor::new(vec![outcome("fn main() {}", "read src/main.rs")]);

        let events = collect(run(
            provider,
            executor,
            request(),
            AgentConfig::default(),
            NoopCancel,
        ));

        assert_eq!(
            events,
            vec![
                AgentEvent::ToolCall {
                    id: "call_0".into(),
                    name: "read_file".into(),
                    summary: r#"read_file {"path":"src/main.rs"}"#.into(),
                },
                AgentEvent::ToolResult {
                    id: "call_0".into(),
                    name: "read_file".into(),
                    ok: true,
                    summary: "read src/main.rs".into(),
                    diff: None,
                },
                AgentEvent::FinalText("fixed it".into()),
            ]
        );

        // The model was called twice; the second request carries the assistant
        // tool_calls message and the tool result.
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].messages.len(), 3);
        assert_eq!(requests[1].messages[1].role, Role::Assistant);
        assert_eq!(
            requests[1].messages[1].tool_calls.as_ref().unwrap(),
            &vec![read]
        );
        assert_eq!(requests[1].messages[2].role, Role::Tool);
        assert_eq!(
            requests[1].messages[2].tool_call_id.as_deref(),
            Some("call_0")
        );
        assert_eq!(requests[1].messages[2].content, "fn main() {}");
    }

    #[test]
    fn final_text_only_when_no_tools() {
        let (provider, requests) =
            FakeProvider::new(vec![Ok(ChatResponse::Text("just an answer".into()))]);
        let executor = FakeExecutor::new(Vec::new());

        let events = collect(run(
            provider,
            executor,
            request(),
            AgentConfig::default(),
            NoopCancel,
        ));

        assert_eq!(events, vec![AgentEvent::FinalText("just an answer".into())]);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn stops_on_turn_budget() {
        // Two turns of tool calls, then the budget (2) is hit before a third.
        let (provider, _requests) = FakeProvider::new(vec![
            Ok(ChatResponse::ToolCalls(vec![call("a", "read_file", "{}")])),
            Ok(ChatResponse::ToolCalls(vec![call("b", "read_file", "{}")])),
        ]);
        let executor = FakeExecutor::new(vec![outcome("1", "s1"), outcome("2", "s2")]);

        let events = collect(run(
            provider,
            executor,
            request(),
            AgentConfig {
                max_turns: 2,
                max_tool_calls: 10,
            },
            NoopCancel,
        ));

        assert_eq!(
            events,
            vec![
                AgentEvent::ToolCall {
                    id: "a".into(),
                    name: "read_file".into(),
                    summary: "read_file {}".into(),
                },
                AgentEvent::ToolResult {
                    id: "a".into(),
                    name: "read_file".into(),
                    ok: true,
                    summary: "s1".into(),
                    diff: None,
                },
                AgentEvent::ToolCall {
                    id: "b".into(),
                    name: "read_file".into(),
                    summary: "read_file {}".into(),
                },
                AgentEvent::ToolResult {
                    id: "b".into(),
                    name: "read_file".into(),
                    ok: true,
                    summary: "s2".into(),
                    diff: None,
                },
                AgentEvent::Error("turn budget exhausted after 2 turns".into()),
            ]
        );
    }

    #[test]
    fn stops_on_tool_budget() {
        // One turn returns two tool calls, but only one is allowed.
        let (provider, _requests) = FakeProvider::new(vec![Ok(ChatResponse::ToolCalls(vec![
            call("a", "read_file", "{}"),
            call("b", "read_file", "{}"),
        ]))]);
        let executor = FakeExecutor::new(vec![outcome("1", "s1")]);

        let events = collect(run(
            provider,
            executor,
            request(),
            AgentConfig {
                max_turns: 10,
                max_tool_calls: 1,
            },
            NoopCancel,
        ));

        assert_eq!(
            events,
            vec![
                AgentEvent::ToolCall {
                    id: "a".into(),
                    name: "read_file".into(),
                    summary: "read_file {}".into(),
                },
                AgentEvent::ToolResult {
                    id: "a".into(),
                    name: "read_file".into(),
                    ok: true,
                    summary: "s1".into(),
                    diff: None,
                },
                AgentEvent::Error("tool budget exhausted after 1 tool calls".into()),
            ]
        );
    }

    #[test]
    fn propagates_provider_error() {
        let (provider, _requests) =
            FakeProvider::new(vec![Err(ProviderError::Http("boom".into()))]);
        let executor = FakeExecutor::new(Vec::new());

        let events = collect(run(
            provider,
            executor,
            request(),
            AgentConfig::default(),
            NoopCancel,
        ));

        assert_eq!(events, vec![AgentEvent::Error("HTTP error: boom".into())]);
    }

    /// A cancel check that fires on its second call.
    struct FlippingCancel {
        calls: Arc<Mutex<u32>>,
    }

    impl CancelCheck for FlippingCancel {
        fn check(&self) -> impl Future<Output = bool> + Send {
            let calls = self.calls.clone();
            async move {
                let n = *calls.lock().unwrap() + 1;
                *calls.lock().unwrap() = n;
                n >= 2
            }
        }
    }

    #[test]
    fn stops_when_cancel_requested() {
        let (provider, _requests) = FakeProvider::new(vec![
            Ok(ChatResponse::ToolCalls(vec![call("a", "read_file", "{}")])),
            Ok(ChatResponse::Text("too late".into())),
        ]);
        let executor = FakeExecutor::new(vec![outcome("1", "s1")]);

        // The first check (before the model call) passes; the second (before
        // the tool runs) fires, so the tool never executes and the run ends
        // with Cancelled.
        let events = collect(run(
            provider,
            executor,
            request(),
            AgentConfig::default(),
            FlippingCancel {
                calls: Arc::new(Mutex::new(0)),
            },
        ));

        assert_eq!(
            events,
            vec![
                AgentEvent::ToolCall {
                    id: "a".into(),
                    name: "read_file".into(),
                    summary: "read_file {}".into(),
                },
                AgentEvent::Cancelled,
            ]
        );
    }
}
