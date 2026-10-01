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
    pub fn apply(&mut self, event: &RunEvent) {
        match event {
            RunEvent::Message { message } | RunEvent::Interim { message } => {
                self.items.push(RunItem::Message(message.clone()));
                if matches!(event, RunEvent::Interim { .. }) {
                    self.text.clear();
                }
            }
            RunEvent::Delta { content } => self.text.push_str(content),
            RunEvent::ToolCall { id, name, summary }
            | RunEvent::PermissionRequest {
                id, name, summary, ..
            }
            | RunEvent::ToolResult {
                id, name, summary, ..
            } => {
                self.text.clear();
                let index = self
                    .items
                    .iter()
                    .position(|item| matches!(item, RunItem::Step(step) if step.id == *id));
                let index = index.unwrap_or_else(|| {
                    self.items.push(RunItem::Step(RunStep {
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
                        _ => None,
                    });
            }
            RunEvent::Done { .. } | RunEvent::Cancelled | RunEvent::Error { .. } => {
                self.finished = Some(event.clone())
            }
        }
    }
}

impl RunEvent {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Message { .. } => "message",
            Self::Delta { .. } => "delta",
            Self::Interim { .. } => "interim",
            Self::ToolCall { .. } => "tool_call",
            Self::PermissionRequest { .. } => "permission_request",
            Self::ToolResult { .. } => "tool_result",
            Self::Telemetry { .. } => "telemetry",
            Self::Done { .. } => "done",
            Self::Cancelled => "cancelled",
            Self::Error { .. } => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
