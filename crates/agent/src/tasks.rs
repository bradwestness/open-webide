//! Child-agent policy shared by every execution host.
pub mod budget;
pub mod executor;
pub mod host;
pub mod model_budget;
pub mod mutations;
pub mod scheduler;
use openwebide_core::{
    ChatMessage, ChatRequest, NewAgentTask, Role, TaskRequest, tasks::MAX_TASK_DEPTH,
};

/// A child inherits configured instructions/model/capabilities, not parent conversation history.
pub fn child_request(
    parent: &ChatRequest,
    task: &NewAgentTask,
    depth: usize,
) -> Result<ChatRequest, String> {
    if depth == 0 || depth > MAX_TASK_DEPTH {
        return Err(format!(
            "Child agents support at most {MAX_TASK_DEPTH} levels of delegation."
        ));
    }
    TaskRequest {
        tasks: vec![task.clone()],
    }
    .validate()?;
    let mut request = parent.clone();
    request.messages = vec![ChatMessage {
        id: 0,
        session_id: 0,
        role: Role::User,
        content: task.prompt.trim().to_owned(),
        created_at: 0,
        tool_calls: None,
        tool_call_id: None,
        usage: None,
    }];
    request.system_prompt = Some(format!(
        "{}\n\nYou are a child agent working on one delegated task. Stay within the task and the inherited project/tool boundaries. Return a concise factual summary of completed work, findings, errors and any unfinished work for the parent agent. Do not assume access to the parent's conversation.",
        parent.system_prompt.as_deref().unwrap_or_default()
    ));
    // A child must not overwrite the parent's durable checklist.
    request.tools.retain(|tool| tool.name != "todo_write");
    if depth == MAX_TASK_DEPTH {
        request.tools.retain(|tool| tool.name != "task");
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::ToolDefinition;
    #[test]
    fn independent_contexts_preserve_host_capabilities_and_limit_recursion() {
        let task = NewAgentTask {
            description: "Inspect dependencies".into(),
            prompt: "List obsolete packages".into(),
        };
        // Local, remote and projectless hosts pass their existing tool capabilities.
        for tools in [
            vec!["read_file", "search_web", "task"],
            vec!["read_file", "search_web", "task"],
            vec!["search_web", "host_info", "task"],
        ] {
            let parent = ChatRequest {
                connection_id: 7,
                model: Some("primary".into()),
                model_settings: Default::default(),
                system_prompt: Some("Trusted project instructions".into()),
                messages: vec![ChatMessage {
                    id: 12,
                    session_id: 9,
                    role: Role::Assistant,
                    content: "Private parent history".into(),
                    created_at: 1,
                    tool_calls: None,
                    tool_call_id: None,
                    usage: None,
                }],
                tools: tools
                    .iter()
                    .map(|name| ToolDefinition {
                        name: (*name).into(),
                        description: String::new(),
                        parameters: serde_json::json!({"type":"object"}),
                    })
                    .collect(),
            };
            let child = child_request(&parent, &task, 1).unwrap();
            assert_eq!(child.messages.len(), 1);
            assert_eq!(child.messages[0].role, Role::User);
            assert_eq!(child.messages[0].content, task.prompt);
            assert_eq!(child.tools, parent.tools);
            assert_eq!(child.model, parent.model);
            assert_eq!(child.connection_id, parent.connection_id);
            assert!(
                child
                    .system_prompt
                    .as_ref()
                    .unwrap()
                    .contains("Trusted project instructions")
            );
            assert!(
                !serde_json::to_string(&child)
                    .unwrap()
                    .contains("Private parent history")
            );
            let deepest = child_request(&parent, &task, MAX_TASK_DEPTH).unwrap();
            assert!(!deepest.tools.iter().any(|tool| tool.name == "task"));
            assert_eq!(
                deepest
                    .tools
                    .iter()
                    .filter(|tool| tool.name == "read_file")
                    .count(),
                usize::from(tools.contains(&"read_file"))
            );
            assert!(child_request(&parent, &task, MAX_TASK_DEPTH + 1).is_err());
        }
    }
}

/// One child transcript and its counters. Hosts provide clock samples, not transition policy.
pub struct TaskRun {
    pub snapshot: openwebide_core::TaskSnapshot,
    completed_usage: openwebide_core::TurnTelemetry,
    pending_usage: Option<openwebide_core::TurnTelemetry>,
    tool_times: std::collections::BTreeMap<String, openwebide_core::ToolTiming>,
    tool_ids: std::collections::BTreeSet<String>,
}
impl TaskRun {
    pub fn new(id: String, parent: String, task: &NewAgentTask, now_ms: u64) -> Self {
        Self {
            snapshot: openwebide_core::TaskSnapshot::new(openwebide_core::AgentTask {
                id,
                parent_tool_call_id: parent,
                description: task.description.clone(),
                status: openwebide_core::TaskStatus::Running,
                timing: Some(openwebide_core::ToolTiming::start(now_ms)),
                telemetry: None,
                tool_count: 0,
                result: None,
            }),
            completed_usage: Default::default(),
            pending_usage: None,
            tool_times: Default::default(),
            tool_ids: Default::default(),
        }
    }
    pub fn started(&mut self, task: &NewAgentTask, now_ms: u64) -> openwebide_core::TaskUpdate {
        self.emit(openwebide_core::TaskEvent::Run {
            event: Box::new(openwebide_core::RunEvent::Message {
                message: Self::message(Role::User, task.prompt.clone(), now_ms, None),
            }),
        })
    }
    fn message(
        role: Role,
        content: String,
        now_ms: u64,
        usage: Option<openwebide_core::TurnTelemetry>,
    ) -> ChatMessage {
        ChatMessage {
            id: 0,
            session_id: 0,
            role,
            content,
            created_at: i64::try_from(now_ms / 1000).unwrap_or(i64::MAX),
            tool_calls: None,
            tool_call_id: None,
            usage,
        }
    }
    fn sum(
        left: openwebide_core::TurnTelemetry,
        right: openwebide_core::TurnTelemetry,
    ) -> openwebide_core::TurnTelemetry {
        openwebide_core::TurnTelemetry {
            prompt_tokens: left.prompt_tokens.saturating_add(right.prompt_tokens),
            completion_tokens: left
                .completion_tokens
                .saturating_add(right.completion_tokens),
            eval_duration_ms: left.eval_duration_ms.saturating_add(right.eval_duration_ms),
            estimated: left.estimated || right.estimated,
            context: None,
        }
    }
    fn finish_turn(&mut self) {
        if let Some(usage) = self.pending_usage.take() {
            self.completed_usage = Self::sum(self.completed_usage, usage);
        }
    }
    fn emit(&mut self, event: openwebide_core::TaskEvent) -> openwebide_core::TaskUpdate {
        let update = openwebide_core::TaskUpdate {
            task: self.snapshot.task.clone(),
            event,
        };
        self.snapshot.apply(&update);
        update
    }
    pub fn observe(
        &mut self,
        event: crate::AgentEvent,
        now_ms: u64,
    ) -> Vec<openwebide_core::TaskUpdate> {
        use crate::AgentEvent;
        use openwebide_core::{RunEvent, TaskEvent, TaskStatus, ToolTiming};
        if self.snapshot.task.status.finished() {
            return Vec::new();
        }
        let mut extra = None;
        let event = match event {
            AgentEvent::TaskUpdate(update) => {
                self.sample(now_ms);
                return vec![self.emit(TaskEvent::Nested { update })];
            }
            AgentEvent::Context(content) => RunEvent::Message {
                message: Self::message(Role::System, content, now_ms, None),
            },
            AgentEvent::Compacted(compaction) => RunEvent::Message {
                message: Self::message(
                    Role::System,
                    compaction.stored_content().unwrap_or_default(),
                    now_ms,
                    None,
                ),
            },
            AgentEvent::TextDelta(content) => RunEvent::Delta { content },
            AgentEvent::ReasoningDelta(content) => RunEvent::ReasoningDelta { content },
            AgentEvent::TurnCalls { text, calls } => {
                let content = openwebide_core::with_reasoning(&self.snapshot.run.reasoning, &text);
                let mut message =
                    Self::message(Role::Assistant, content, now_ms, self.pending_usage);
                message.tool_calls = Some(calls);
                self.finish_turn();
                RunEvent::Interim { message }
            }
            AgentEvent::Telemetry(usage) => {
                self.pending_usage = Some(usage);
                self.snapshot.task.telemetry = Some(Self::sum(self.completed_usage, usage));
                RunEvent::Telemetry { usage }
            }
            AgentEvent::ToolCall { id, name, summary } => {
                self.snapshot.task.status = TaskStatus::Running;
                self.tool_ids.insert(id.clone());
                let timing = *self
                    .tool_times
                    .entry(id.clone())
                    .or_insert_with(|| ToolTiming::start(now_ms));
                extra = Some(RunEvent::ToolTiming {
                    id: id.clone(),
                    timing,
                });
                RunEvent::ToolCall { id, name, summary }
            }
            AgentEvent::PermissionRequest {
                id,
                name,
                summary,
                diff,
                note,
            } => {
                self.snapshot.task.status = TaskStatus::WaitingForApproval;
                self.tool_ids.insert(id.clone());
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
                self.snapshot.task.status = TaskStatus::Running;
                if let Some(timing) = self.tool_times.get_mut(&id) {
                    *timing = timing.sample(now_ms, true);
                    extra = Some(RunEvent::ToolTiming {
                        id: id.clone(),
                        timing: *timing,
                    });
                }
                RunEvent::ToolResult {
                    id,
                    name,
                    ok,
                    summary,
                    diff,
                }
            }
            AgentEvent::FileCheckpoint { id, diff } => {
                self.sample(now_ms);
                return vec![self.emit(TaskEvent::FileCheckpoint { id, diff })];
            }
            AgentEvent::ProjectCheckpoint { id, checkpoint } => {
                self.sample(now_ms);
                return vec![self.emit(TaskEvent::ProjectCheckpoint { id, checkpoint })];
            }
            AgentEvent::FinalText(text) => {
                let content = openwebide_core::with_reasoning(&self.snapshot.run.reasoning, &text);
                let message = Self::message(Role::Assistant, content, now_ms, self.pending_usage);
                self.finish_turn();
                self.snapshot.task.status = TaskStatus::Completed;
                self.snapshot.task.result =
                    Some(openwebide_core::strip_reasoning(&text).to_owned());
                RunEvent::Done { message }
            }
            AgentEvent::Error(message) => {
                self.snapshot.task.status = TaskStatus::Failed;
                self.snapshot.task.result = Some(message.clone());
                RunEvent::Error { message }
            }
            AgentEvent::Cancelled => {
                self.snapshot.task.status = TaskStatus::Cancelled;
                self.snapshot.task.result = Some("Cancelled".into());
                RunEvent::Cancelled
            }
        };
        self.snapshot.task.tool_count = self.tool_ids.len();
        self.sample(now_ms);
        let mut updates = Vec::new();
        if self.snapshot.task.status.finished() {
            let final_times: Vec<_> = self
                .tool_times
                .iter_mut()
                .filter(|(_, timing)| !timing.finished)
                .map(|(id, timing)| {
                    *timing = timing.sample(now_ms, true);
                    RunEvent::ToolTiming {
                        id: id.clone(),
                        timing: *timing,
                    }
                })
                .collect();
            for event in final_times {
                updates.push(self.emit(TaskEvent::Run {
                    event: Box::new(event),
                }));
            }
        }
        updates.push(self.emit(TaskEvent::Run {
            event: Box::new(event),
        }));
        if let Some(event) = extra {
            updates.push(self.emit(TaskEvent::Run {
                event: Box::new(event),
            }));
        }
        updates
    }
    fn sample(&mut self, now_ms: u64) {
        self.snapshot.task.timing = self
            .snapshot
            .task
            .timing
            .map(|timing| timing.sample(now_ms, self.snapshot.task.status.finished()));
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    use crate::AgentEvent;
    use openwebide_core::{RunItem, TaskStatus, TurnTelemetry};
    fn task() -> NewAgentTask {
        NewAgentTask {
            description: "Inspect dependencies".into(),
            prompt: "List obsolete packages".into(),
        }
    }
    #[test]
    fn child_approval_timing_usage_and_cancellation_are_durable_and_replay_safe() {
        let mut run = TaskRun::new("parent.task1".into(), "parent".into(), &task(), 1000);
        run.started(&task(), 1000);
        run.observe(
            AgentEvent::PermissionRequest {
                id: "child-tool".into(),
                name: "write_file".into(),
                summary: "Write file".into(),
                diff: None,
                note: None,
            },
            2000,
        );
        assert_eq!(run.snapshot.task.status, TaskStatus::WaitingForApproval);
        run.observe(
            AgentEvent::ToolCall {
                id: "child-tool".into(),
                name: "write_file".into(),
                summary: "Write file".into(),
            },
            5000,
        );
        assert_eq!(run.snapshot.task.tool_count, 1);
        run.observe(
            AgentEvent::Telemetry(TurnTelemetry {
                prompt_tokens: 40,
                completion_tokens: 8,
                ..Default::default()
            }),
            5100,
        );
        run.observe(
            AgentEvent::Telemetry(TurnTelemetry {
                prompt_tokens: 40,
                completion_tokens: 10,
                ..Default::default()
            }),
            5200,
        );
        let updates = run.observe(AgentEvent::Cancelled, 6000);
        assert_eq!(run.snapshot.task.telemetry.unwrap().context_tokens(), 50);
        assert_eq!(run.snapshot.task.timing.unwrap().elapsed_ms, 5000);
        assert_eq!(run.snapshot.task.status, TaskStatus::Cancelled);
        let step = run
            .snapshot
            .run
            .items
            .iter()
            .find_map(|item| match item {
                RunItem::Step(step) => Some(step),
                RunItem::Message(_) => None,
            })
            .unwrap();
        assert_eq!(
            step.timing.unwrap().elapsed_ms,
            1000,
            "Approval wait is excluded from tool execution"
        );
        assert!(step.timing.unwrap().finished);
        assert!(!step.awaiting_permission);
        let stored = serde_json::to_string(&run.snapshot).unwrap();
        let mut loaded: openwebide_core::TaskSnapshot = serde_json::from_str(&stored).unwrap();
        let mut stale = updates.last().unwrap().clone();
        stale.task.status = TaskStatus::Running;
        stale.task.timing = Some(openwebide_core::ToolTiming::start(1000));
        stale.event = openwebide_core::TaskEvent::Run {
            event: Box::new(openwebide_core::RunEvent::Delta {
                content: "late text".into(),
            }),
        };
        loaded.apply(&stale);
        assert_eq!(loaded, run.snapshot);
        assert!(
            run.observe(AgentEvent::FinalText("too late".into()), 7000)
                .is_empty()
        );
    }
    #[test]
    fn usage_counts_each_model_turn_once_and_parent_result_excludes_reasoning() {
        let mut run = TaskRun::new("parent.task1".into(), "parent".into(), &task(), 1000);
        run.started(&task(), 1000);
        run.observe(
            AgentEvent::Telemetry(TurnTelemetry {
                prompt_tokens: 40,
                completion_tokens: 10,
                ..Default::default()
            }),
            1100,
        );
        run.observe(
            AgentEvent::TurnCalls {
                text: String::new(),
                calls: Vec::new(),
            },
            1200,
        );
        run.observe(
            AgentEvent::Telemetry(TurnTelemetry {
                prompt_tokens: 60,
                completion_tokens: 20,
                ..Default::default()
            }),
            1300,
        );
        run.observe(
            AgentEvent::FinalText("<think>private reasoning</think>Package findings".into()),
            1400,
        );
        assert_eq!(run.snapshot.task.telemetry.unwrap().context_tokens(), 130);
        assert_eq!(
            run.snapshot.task.result.as_deref(),
            Some("Package findings")
        );
        assert_eq!(run.snapshot.task.status, TaskStatus::Completed);
    }
}
