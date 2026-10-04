//! Shared child-agent requests and run metadata.
use crate::{ToolTiming, TurnTelemetry};
use serde::{Deserialize, Serialize};

pub const MAX_TASKS_PER_REQUEST: usize = 4;
pub const MAX_PARALLEL_TASKS: usize = 4;
pub const MAX_TASK_DEPTH: usize = 2;
pub const MAX_TASK_PROMPT_CHARS: usize = 16_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAgentTask {
    pub description: String,
    pub prompt: String,
}

/// A single task tool call may dispatch independent work in parallel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRequest {
    pub tasks: Vec<NewAgentTask>,
}
impl TaskRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.tasks.is_empty() || self.tasks.len() > MAX_TASKS_PER_REQUEST {
            return Err(format!(
                "Provide 1–{MAX_TASKS_PER_REQUEST} independent tasks."
            ));
        }
        for task in &self.tasks {
            let description = task.description.trim();
            if description.is_empty()
                || description.chars().count() > 80
                || description.chars().any(char::is_control)
            {
                return Err("Task descriptions must contain 1–80 characters on one line.".into());
            }
            let prompt = task.prompt.trim();
            if prompt.is_empty() || prompt.chars().count() > MAX_TASK_PROMPT_CHARS {
                return Err(format!(
                    "Task prompts must contain 1–{MAX_TASK_PROMPT_CHARS} characters."
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Queued,
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
}
impl TaskStatus {
    pub const fn finished(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTask {
    pub id: String,
    pub parent_tool_call_id: String,
    pub description: String,
    pub status: TaskStatus,
    pub timing: Option<ToolTiming>,
    pub telemetry: Option<TurnTelemetry>,
    pub tool_count: usize,
    pub result: Option<String>,
}

/// Scope every child tool/approval identifier to its parent call and task position.
pub fn task_run_id(parent_tool_call_id: &str, index: usize) -> String {
    format!("{parent_tool_call_id}.task{}", index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_requests_reject_empty_batches_and_unbounded_model_inputs() {
        let task = NewAgentTask {
            description: "Inspect package dependencies".into(),
            prompt: "Find obsolete packages and explain compatible upgrades.".into(),
        };
        let valid = TaskRequest {
            tasks: vec![task.clone(); MAX_TASKS_PER_REQUEST],
        };
        assert!(valid.validate().is_ok());
        for tasks in [
            Vec::new(),
            vec![task.clone(); MAX_TASKS_PER_REQUEST + 1],
            vec![NewAgentTask {
                prompt: "x".repeat(MAX_TASK_PROMPT_CHARS + 1),
                ..task.clone()
            }],
            vec![NewAgentTask {
                description: "Title\nInjected text".into(),
                ..task
            }],
        ] {
            assert!(TaskRequest { tasks }.validate().is_err());
        }
        assert_ne!(
            task_run_id("prompt-1.tool-1", 0),
            task_run_id("prompt-1.tool-2", 0)
        );
        assert_ne!(
            task_run_id("prompt-1.tool-1", 0),
            task_run_id("prompt-1.tool-1", 1)
        );
    }
}

/// Progress is scoped to a child context; checkpoints retain the parent's rewind boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskEvent {
    Run {
        event: Box<crate::RunEvent>,
    },
    FileCheckpoint {
        id: String,
        diff: crate::FileDiff,
    },
    ProjectCheckpoint {
        id: String,
        checkpoint: crate::rewind::ProjectCheckpoint,
    },
    Nested {
        update: Box<TaskUpdate>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskUpdate {
    pub task: AgentTask,
    pub event: TaskEvent,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task: AgentTask,
    pub run: crate::RunSnapshot,
    #[serde(default)]
    pub children: Vec<TaskSnapshot>,
}
impl TaskSnapshot {
    pub fn new(task: AgentTask) -> Self {
        Self {
            task,
            run: Default::default(),
            children: Vec::new(),
        }
    }
    pub fn apply(&mut self, update: &TaskUpdate) {
        if self.task.id != update.task.id
            || self.task.parent_tool_call_id != update.task.parent_tool_call_id
        {
            return;
        }
        if self.run.finished.is_some() {
            if let TaskEvent::Run { event } = &update.event
                && matches!(**event, crate::RunEvent::ToolTiming { .. })
            {
                self.run.apply(event);
            }
            return;
        }
        let timing = match (self.task.timing, update.task.timing) {
            (Some(previous), Some(incoming)) => match previous.merge(incoming) {
                Ok(timing) => Some(timing),
                Err(_) => return,
            },
            (previous, incoming) => incoming.or(previous),
        };
        self.task.clone_from(&update.task);
        self.task.timing = timing;
        match &update.event {
            TaskEvent::Run { event } => {
                self.run.apply(event);
                if self.run.finished.is_some() {
                    let now = self.task.timing.map_or(0, |timing| {
                        timing.started_at_ms.saturating_add(timing.elapsed_ms)
                    });
                    for child in &mut self.children {
                        child.stop(now);
                    }
                    for item in &mut self.run.items {
                        if let crate::RunItem::Step(step) = item {
                            step.awaiting_permission = false;
                            step.timing = step.timing.map(|timing| timing.sample(now, true));
                            if step.result.is_none() {
                                step.result = Some(crate::ToolStepResultWire {
                                    ok: false,
                                    summary: "Child stopped before a result was recorded".into(),
                                    diff: None,
                                });
                            }
                        }
                    }
                }
            }
            TaskEvent::Nested { update } => {
                if let Some(child) = self
                    .children
                    .iter_mut()
                    .find(|child| child.task.id == update.task.id)
                {
                    child.apply(update);
                } else {
                    let mut child = Self::new(update.task.clone());
                    child.apply(update);
                    self.children.push(child);
                }
            }
            TaskEvent::FileCheckpoint { .. } | TaskEvent::ProjectCheckpoint { .. } => (),
        }
    }
    pub fn total_tokens(&self) -> usize {
        self.children.iter().fold(
            self.task
                .telemetry
                .map_or(0, |usage| usage.context_tokens()),
            |total, child| total.saturating_add(child.total_tokens()),
        )
    }
}

/// Distinguish persisted child tool steps from the parent's model history.
/// Every delegation segment preserves the root anchor and bounded task/depth IDs.
pub fn task_step_scope(id: &str) -> Option<&str> {
    let parts: Vec<_> = id.split('.').collect();
    if parts.len() < 3 || parts.len() > MAX_TASK_DEPTH * 2 + 1 || parts.len() % 2 != 1 {
        return None;
    }
    let (anchor, _, _) = crate::parse_step_id(parts[0])?;
    for pair in parts[1..].as_chunks::<2>().0 {
        let index = pair[0].strip_prefix("task")?.parse::<usize>().ok()?;
        if !(1..=MAX_TASKS_PER_REQUEST).contains(&index)
            || crate::parse_step_id(pair[1])?.0 != anchor
        {
            return None;
        }
    }
    Some(id.rsplit_once('.')?.0)
}

#[cfg(test)]
mod history_tests {
    use super::*;
    #[test]
    fn scoped_steps_are_validated_without_changing_root_turn_parsing() {
        let id = "a12t1c0.task1.a12t3c0";
        assert_eq!(task_step_scope(id), Some("a12t1c0.task1"));
        assert!(crate::parse_step_id(id).is_none());
        assert!(task_step_scope("a12t1c0.task1.a12t3c0.task4.a12t2c0").is_some());
        for invalid in [
            "a12t1c0",
            "a12t1c0.task5.a12t1c0",
            "a12t1c0.task1.a13t1c0",
            "a12t1c0.task1.a12t1c0.task1.a12t1c0.task1.a12t1c0",
        ] {
            assert!(task_step_scope(invalid).is_none(), "{invalid}");
        }
    }
    #[test]
    fn child_results_do_not_displace_parent_task_summary() {
        let message: crate::ChatMessage = serde_json::from_value(serde_json::json!({
            "id":12,"session_id":1,"role":"assistant","content":"Delegating", "created_at":0,
            "tool_calls":[{"id":"wire-parent","name":"task","arguments":"{}"}]
        }))
        .unwrap();
        let step = |id: &str, result: &str| {
            serde_json::from_value::<crate::ToolStep>(serde_json::json!({
                "tool_call_id":id,"name":"task","summary":"Task","ok":true,
                "result_summary":result,"anchor_message_id":12
            }))
            .unwrap()
        };
        let history = crate::tool_history(
            vec![message],
            &[
                step("a12t1c0", "Parent summary"),
                step("a12t1c0.task1.a12t1c0", "Child private result"),
            ],
        );
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].content, "Parent summary");
        assert_eq!(history[1].tool_call_id.as_deref(), Some("wire-parent"));
    }
}

/// A child tree belongs to the assistant turn that requested its parent tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskHistory {
    pub anchor_message_id: i64,
    pub snapshot: TaskSnapshot,
}

impl TaskSnapshot {
    pub fn find_task(&self, id: &str) -> Option<&Self> {
        if self.task.id == id {
            Some(self)
        } else {
            self.children.iter().find_map(|child| child.find_task(id))
        }
    }
    pub fn usages(&self) -> Vec<TurnTelemetry> {
        let mut usages = self.task.telemetry.into_iter().collect::<Vec<_>>();
        for child in &self.children {
            usages.extend(child.usages());
        }
        usages
    }
}

impl TaskUpdate {
    pub fn leaf_update(&self) -> &Self {
        match &self.event {
            TaskEvent::Nested { update } => update.leaf_update(),
            _ => self,
        }
    }
    pub fn leaf_event(&self) -> &TaskEvent {
        &self.leaf_update().event
    }
}
impl TaskSnapshot {
    pub fn pending_permission(&self) -> Option<(String, String)> {
        if self.run.finished.is_some() {
            return None;
        }
        self.run
            .items
            .iter()
            .find_map(|item| match item {
                crate::RunItem::Step(step) if step.awaiting_permission && step.result.is_none() => {
                    Some((step.id.clone(), step.name.clone()))
                }
                _ => None,
            })
            .or_else(|| self.children.iter().find_map(Self::pending_permission))
    }
    pub fn total_tools(&self) -> usize {
        self.children
            .iter()
            .fold(self.task.tool_count, |total, child| {
                total.saturating_add(child.total_tools())
            })
    }
    pub fn contains_tool(&self, id: &str) -> bool {
        self.run
            .items
            .iter()
            .any(|item| matches!(item, crate::RunItem::Step(step) if step.id == id))
            || self.children.iter().any(|child| child.contains_tool(id))
    }
    pub fn clear_permission(&mut self, id: &str) {
        for item in &mut self.run.items {
            if let crate::RunItem::Step(step) = item
                && step.id == id
            {
                step.awaiting_permission = false;
            }
        }
        for child in &mut self.children {
            child.clear_permission(id);
        }
    }
}

impl TaskSnapshot {
    /// Freeze unfinished descendants when their owning run stops unexpectedly.
    pub fn stop(&mut self, now_ms: u64) {
        for child in &mut self.children {
            child.stop(now_ms);
        }
        if self.run.finished.is_some() {
            return;
        }
        self.task.status = TaskStatus::Cancelled;
        self.task.result = Some("Parent run stopped".into());
        self.task.timing = self.task.timing.map(|timing| timing.sample(now_ms, true));
        self.run.finished = Some(crate::RunEvent::Cancelled);
        for item in &mut self.run.items {
            if let crate::RunItem::Step(step) = item {
                step.awaiting_permission = false;
                step.timing = step.timing.map(|timing| timing.sample(now_ms, true));
                if step.result.is_none() {
                    step.result = Some(crate::ToolStepResultWire {
                        ok: false,
                        summary: "Interrupted before a result was recorded".into(),
                        diff: None,
                    });
                }
            }
        }
    }
}
