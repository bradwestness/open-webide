//! Shared chat and agent run events and snapshots.

use crate::{ChatMessage, FileDiff, TurnTelemetry};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    Message {
        message: ChatMessage,
    },
    Delta {
        content: String,
    },
    ReasoningDelta {
        content: String,
    },
    Interim {
        message: ChatMessage,
    },
    ToolCall {
        id: String,
        name: String,
        summary: String,
    },
    PermissionRequest {
        id: String,
        name: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<FileDiff>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    ToolResult {
        id: String,
        name: String,
        ok: bool,
        summary: String,
        diff: Option<FileDiff>,
    },
    ToolTiming {
        id: String,
        timing: crate::ToolTiming,
    },
    Telemetry {
        usage: TurnTelemetry,
    },
    Done {
        message: ChatMessage,
    },
    Cancelled,
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RunSnapshot {
    pub items: Vec<RunItem>,
    pub text: String,
    #[serde(default)]
    pub reasoning: String,
    pub telemetry: Option<TurnTelemetry>,
    #[serde(default)]
    pub telemetry_after_message_id: Option<i64>,
    pub finished: Option<RunEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunItem {
    Message(ChatMessage),
    Step(RunStep),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<crate::ToolTiming>,
    pub id: String,
    pub name: String,
    pub summary: String,
    pub awaiting_permission: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<Box<FileDiff>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub result: Option<ToolStepResultWire>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStepResultWire {
    pub ok: bool,
    pub summary: String,
    pub diff: Option<FileDiff>,
}

impl RunSnapshot {
    pub fn refresh_tool_timings(&mut self, now_ms: u64) {
        for item in &mut self.items {
            if let RunItem::Step(step) = item {
                step.timing = step
                    .timing
                    .filter(|timing| {
                        timing.finished || (self.finished.is_none() && step.result.is_none())
                    })
                    .map(|timing| timing.sample(now_ms, false));
            }
        }
    }

    pub fn apply(&mut self, event: &RunEvent) {
        match event {
            RunEvent::ToolTiming { id, timing } => {
                if let Some(RunItem::Step(step)) = self
                    .items
                    .iter_mut()
                    .find(|item| matches!(item, RunItem::Step(step) if step.id == *id))
                    && let Ok(timing) = step
                        .timing
                        .map_or(Ok(*timing), |previous| previous.merge(*timing))
                {
                    step.timing = Some(timing);
                }
            }
            RunEvent::Message { message } | RunEvent::Interim { message } => {
                self.items.push(RunItem::Message(message.clone()));
                if matches!(event, RunEvent::Interim { .. }) {
                    self.text.clear();
                    self.reasoning.clear();
                }
            }
            RunEvent::Delta { content } => self.text.push_str(content),
            RunEvent::ReasoningDelta { content } => self.reasoning.push_str(content),
            RunEvent::ToolCall { id, name, summary }
            | RunEvent::PermissionRequest {
                id, name, summary, ..
            }
            | RunEvent::ToolResult {
                id, name, summary, ..
            } => {
                self.text.clear();
                self.reasoning.clear();
                let index = self
                    .items
                    .iter()
                    .position(|item| matches!(item, RunItem::Step(step) if step.id == *id));
                let index = index.unwrap_or_else(|| {
                    self.items.push(RunItem::Step(RunStep {
                        timing: None,
                        id: id.clone(),
                        name: name.clone(),
                        summary: summary.clone(),
                        awaiting_permission: false,
                        result: None,
                        diff: None,
                        note: None,
                    }));
                    self.items.len() - 1
                });
                if let RunItem::Step(step) = &mut self.items[index] {
                    step.name.clone_from(name);
                    match event {
                        RunEvent::ToolResult {
                            ok, summary, diff, ..
                        } => {
                            step.awaiting_permission = false;
                            step.diff = None;
                            step.note = None;
                            step.result = Some(ToolStepResultWire {
                                ok: *ok,
                                summary: summary.clone(),
                                diff: diff.clone(),
                            });
                        }
                        RunEvent::PermissionRequest { diff, note, .. } => {
                            step.summary.clone_from(summary);
                            step.awaiting_permission = true;
                            step.diff = diff.clone().map(Box::new);
                            step.note.clone_from(note);
                        }
                        _ => {
                            step.summary.clone_from(summary);
                            step.awaiting_permission =
                                matches!(event, RunEvent::PermissionRequest { .. });
                        }
                    }
                }
            }
            RunEvent::Telemetry { usage } => {
                self.telemetry = Some(*usage);
                self.telemetry_after_message_id =
                    self.items.iter().rev().find_map(|item| match item {
                        RunItem::Message(message) => Some(message.id),
                        RunItem::Step(_) => None,
                    });
            }
            RunEvent::Done { .. } | RunEvent::Cancelled | RunEvent::Error { .. } => {
                self.finished = Some(event.clone());
            }
        }
    }
}

impl RunEvent {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Message { .. } => "message",
            Self::Delta { .. } => "delta",
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::Interim { .. } => "interim",
            Self::ToolCall { .. } => "tool_call",
            Self::PermissionRequest { .. } => "permission_request",
            Self::ToolResult { .. } => "tool_result",
            Self::ToolTiming { .. } => "tool_timing",
            Self::Telemetry { .. } => "telemetry",
            Self::Done { .. } => "done",
            Self::Cancelled => "cancelled",
            Self::Error { .. } => "error",
        }
    }
}

/// Project-relative execution root, shared by backend planning and frontend host selection.
pub fn execution_root(project: &crate::Project) -> Option<String> {
    // Host capability boundary: browser directory handles have no server filesystem root.
    if project.mode == crate::WorkspaceMode::Local {
        return None;
    }
    project
        .path
        .as_deref()
        .and_then(|path| crate::vfs::workspace_path(path).ok())
        .map(|path| if path.is_empty() { ".".into() } else { path })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterruptedRun {
    pub anchor_id: i64,
    pub first_turn: usize,
}

pub fn interrupted_run(items: &[crate::RunItem]) -> Option<InterruptedRun> {
    use crate::Role;
    let last = items.iter().rev().find_map(|item| match item {
        crate::RunItem::Message(message) if message.id > 0 && message.role != Role::System => {
            Some(message)
        }
        _ => None,
    })?;
    if last.role != Role::User && !(last.role == Role::Assistant && last.tool_calls.is_some()) {
        return None;
    }
    let anchor_id = items.iter().rev().find_map(|item| match item {
        crate::RunItem::Message(message) if message.id > 0 && message.role == Role::User => {
            Some(message.id)
        }
        _ => None,
    })?;
    if last.role == Role::Assistant {
        let calls = last.tool_calls.as_ref()?;
        let last_position = items.iter().rposition(
            |item| matches!(item, crate::RunItem::Message(message) if message.id == last.id),
        )?;
        let steps: Vec<_> = items[last_position + 1..]
            .iter()
            .filter_map(|item| match item {
                crate::RunItem::Step(crate::RunStep {
                    id,
                    result,
                    awaiting_permission,
                    ..
                }) if crate::parse_step_id(id)
                    .is_some_and(|(anchor, _, _)| anchor == anchor_id) =>
                {
                    Some((result, awaiting_permission))
                }
                _ => None,
            })
            .collect();
        if calls.is_empty()
            || steps.len() != calls.len()
            || steps
                .iter()
                .any(|(result, awaiting)| result.is_none() || **awaiting)
        {
            return None;
        }
    }
    let highest_turn = items
        .iter()
        .filter_map(|item| match item {
            crate::RunItem::Step(crate::RunStep { id, .. }) => crate::parse_step_id(id)
                .filter(|(anchor, _, _)| *anchor == anchor_id)
                .map(|(_, turn, _)| turn),
            crate::RunItem::Message(_) => None,
        })
        .max()
        .unwrap_or(0);
    Some(InterruptedRun {
        anchor_id,
        first_turn: highest_turn.checked_add(1)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_refreshes_running_timing_and_retains_completed_duration_on_replay() {
        let mut snapshot = RunSnapshot::default();
        snapshot.apply(&RunEvent::ToolCall {
            id: "tool".into(),
            name: "host_info".into(),
            summary: "host".into(),
        });
        let timing = crate::ToolTiming::start(1000);
        snapshot.apply(&RunEvent::ToolTiming {
            id: "tool".into(),
            timing,
        });
        snapshot.refresh_tool_timings(4200);
        let RunItem::Step(step) = &snapshot.items[0] else {
            panic!("missing step");
        };
        assert_eq!(step.timing.unwrap().elapsed_ms, 3200);
        let completed = timing.sample(4300, true);
        snapshot.apply(&RunEvent::ToolTiming {
            id: "tool".into(),
            timing: completed,
        });
        snapshot.apply(&RunEvent::ToolTiming {
            id: "tool".into(),
            timing,
        });
        snapshot.refresh_tool_timings(9999);
        let RunItem::Step(step) = &snapshot.items[0] else {
            panic!("missing step");
        };
        assert_eq!(step.timing, Some(completed));
        let decoded: RunSnapshot =
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn snapshot_does_not_restart_partial_timing_after_completion_or_interruption() {
        for completed in [false, true] {
            let mut snapshot = RunSnapshot::default();
            snapshot.apply(&RunEvent::ToolCall {
                id: "t".into(),
                name: "shell".into(),
                summary: "command".into(),
            });
            snapshot.apply(&RunEvent::ToolTiming {
                id: "t".into(),
                timing: crate::ToolTiming::start(1000),
            });
            if completed {
                snapshot.apply(&RunEvent::ToolResult {
                    id: "t".into(),
                    name: "shell".into(),
                    ok: true,
                    summary: "done".into(),
                    diff: None,
                });
            } else {
                snapshot.apply(&RunEvent::Cancelled);
            }
            snapshot.refresh_tool_timings(9999);
            let RunItem::Step(step) = &snapshot.items[0] else {
                panic!("missing step")
            };
            assert!(step.timing.is_none());
        }
    }

    #[test]
    fn legacy_permission_requests_and_snapshots_default_the_preview() {
        let event: RunEvent = serde_json::from_str(
            r#"{"kind":"permission_request","id":"t","name":"write_file","summary":"write file"}"#,
        )
        .unwrap();
        assert!(matches!(
            event,
            RunEvent::PermissionRequest {
                diff: None,
                note: None,
                ..
            }
        ));
        let step: RunStep = serde_json::from_str(r#"{"id":"t","name":"write_file","summary":"write file","awaiting_permission":true,"result":null}"#).unwrap();
        assert_eq!(step.diff, None);
        assert_eq!(step.note, None);
    }

    #[test]
    fn snapshot_keeps_preview_until_completion() {
        let diff = FileDiff {
            path: "file".into(),
            old: Some("old".into()),
            new: "new".into(),
            old_unavailable: false,
            backup_path: None,
        };
        let mut snapshot = RunSnapshot::default();
        snapshot.apply(&RunEvent::PermissionRequest {
            id: "t".into(),
            name: "write_file".into(),
            summary: "write file".into(),
            diff: Some(diff.clone()),
            note: Some("preview".into()),
        });
        let RunItem::Step(step) = &snapshot.items[0] else {
            panic!("missing step")
        };
        assert_eq!(step.diff.as_deref(), Some(&diff));
        assert_eq!(step.note.as_deref(), Some("preview"));
        assert!(step.awaiting_permission);
        snapshot.apply(&RunEvent::ToolResult {
            id: "t".into(),
            name: "write_file".into(),
            ok: false,
            summary: "denied".into(),
            diff: None,
        });
        let RunItem::Step(step) = &snapshot.items[0] else {
            panic!("missing step")
        };
        assert_eq!(step.diff, None);
        assert_eq!(step.note, None);
        assert!(!step.awaiting_permission);
    }
    #[test]
    fn snapshot_retains_live_reasoning_and_clears_it_after_tool_turn() {
        let mut snapshot = RunSnapshot::default();
        snapshot.apply(&RunEvent::ReasoningDelta {
            content: "r".into(),
        });
        snapshot.apply(&RunEvent::Delta {
            content: "answer".into(),
        });
        assert_eq!(snapshot.reasoning, "r");
        assert_eq!(snapshot.text, "answer");
        snapshot.apply(&RunEvent::ToolCall {
            id: "t".into(),
            name: "read_file".into(),
            summary: "read".into(),
        });
        assert!(snapshot.reasoning.is_empty());
        assert!(snapshot.text.is_empty());
    }
}
