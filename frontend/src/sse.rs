use openwebide_core::{ChatMessage, FileDiff, TurnTelemetry};

/// A server-sent event from the message stream.
#[derive(Clone, Debug, PartialEq)]
pub enum SseEvent {
    /// A completed message, emitted immediately for the user message and
    /// once at the end of a non-streaming run.
    Message(ChatMessage),
    /// A token delta appended to the in-progress assistant reply.
    Delta(String),
    /// The agent requested a tool call.
    ToolCall {
        id: String,
        name: String,
        summary: String,
    },
    /// A gated tool call is waiting for the user's approval; a
    /// `ToolResult` follows once the decision is in (either way).
    PermissionRequest {
        id: String,
        name: String,
        summary: String,
    },
    /// A tool call finished.
    ToolResult {
        id: String,
        #[allow(dead_code)]
        name: String,
        ok: bool,
        summary: String,
        diff: Option<FileDiff>,
    },
    /// The final, persisted assistant reply.
    Done(ChatMessage),
    /// Persisted text preceding tool calls; the run continues.
    Interim(ChatMessage),
    /// Telemetry metrics for the turn.
    Telemetry(TurnTelemetry),
    /// The run was cancelled; the stream ends after this.
    Cancelled,
    /// A provider or persistence error; the stream ends after this.
    Error(String),
}

impl From<openwebide_core::RunEvent> for SseEvent {
    fn from(event: openwebide_core::RunEvent) -> Self {
        use openwebide_core::RunEvent;
        match event {
            RunEvent::Message { message } => Self::Message(message),
            RunEvent::Delta { content } => Self::Delta(content),
            RunEvent::Interim { message } => Self::Interim(message),
            RunEvent::ToolCall { id, name, summary } => Self::ToolCall { id, name, summary },
            RunEvent::PermissionRequest { id, name, summary } => {
                Self::PermissionRequest { id, name, summary }
            }
            RunEvent::ToolResult {
                id,
                name,
                ok,
                summary,
                diff,
            } => Self::ToolResult {
                id,
                name,
                ok,
                summary,
                diff,
            },
            RunEvent::Telemetry { usage } => Self::Telemetry(usage),
            RunEvent::Done { message } => Self::Done(message),
            RunEvent::Cancelled => Self::Cancelled,
            RunEvent::Error { message } => Self::Error(message),
        }
    }
}

/// Parse one SSE frame (`event: <name>\ndata: <json>\n\n`) into an [`SseEvent`].
pub fn parse_frame(frame: &str) -> Option<SseEvent> {
    let mut event = "message";
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(v) = line.strip_prefix("event: ") {
            event = v.trim();
        } else if let Some(v) = line.strip_prefix("data: ") {
            data.push_str(v.trim());
        }
    }
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;
    parse_event(event, value)
}

pub fn parse_event(event: &str, value: serde_json::Value) -> Option<SseEvent> {
    Some(match event {
        "message" => SseEvent::Message(serde_json::from_value(value).ok()?),
        "delta" => SseEvent::Delta(value.get("content")?.as_str()?.to_string()),
        "tool_call" => SseEvent::ToolCall {
            id: value.get("id")?.as_str()?.to_string(),
            name: value.get("name")?.as_str()?.to_string(),
            summary: value.get("summary")?.as_str()?.to_string(),
        },
        "permission_request" => SseEvent::PermissionRequest {
            id: value.get("id")?.as_str()?.to_string(),
            name: value.get("name")?.as_str()?.to_string(),
            summary: value.get("summary")?.as_str()?.to_string(),
        },
        "tool_result" => SseEvent::ToolResult {
            id: value.get("id")?.as_str()?.to_string(),
            name: value.get("name")?.as_str()?.to_string(),
            ok: value.get("ok")?.as_bool().unwrap_or(false),
            summary: value.get("summary")?.as_str()?.to_string(),
            diff: value
                .get("diff")
                .and_then(|d| serde_json::from_value(d.clone()).ok().flatten()),
        },
        "interim" => SseEvent::Interim(serde_json::from_value(value).ok()?),
        "done" => SseEvent::Done(serde_json::from_value(value).ok()?),
        "telemetry" => SseEvent::Telemetry(serde_json::from_value(value).ok()?),
        "cancelled" => SseEvent::Cancelled,
        "error" => SseEvent::Error(value.get("error")?.as_str()?.to_string()),
        _ => return None,
    })
}

pub struct FrameBuffer {
    pending: String,
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self {
            pending: String::new(),
        }
    }

    pub fn push(&mut self, data: &str) -> Vec<String> {
        self.pending.push_str(data);
        let mut frames = Vec::new();
        while let Some(pos) = self.pending.find("\n\n") {
            let frame = self.pending[..pos].to_string();
            self.pending.drain(..pos + 2);
            frames.push(frame);
        }
        frames
    }
}

impl Default for FrameBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::{Role, RunEvent};

    #[test]
    fn typed_run_events_match_sse_events() {
        let message = ChatMessage {
            id: 2,
            session_id: 1,
            role: Role::Assistant,
            content: "reply".into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        };
        let pairs = vec![
            (
                RunEvent::Message {
                    message: message.clone(),
                },
                SseEvent::Message(message.clone()),
            ),
            (
                RunEvent::Delta {
                    content: "a".into(),
                },
                SseEvent::Delta("a".into()),
            ),
            (
                RunEvent::Interim {
                    message: message.clone(),
                },
                SseEvent::Interim(message.clone()),
            ),
            (
                RunEvent::Done {
                    message: message.clone(),
                },
                SseEvent::Done(message),
            ),
            (
                RunEvent::ToolCall {
                    id: "t".into(),
                    name: "read_file".into(),
                    summary: "file".into(),
                },
                SseEvent::ToolCall {
                    id: "t".into(),
                    name: "read_file".into(),
                    summary: "file".into(),
                },
            ),
            (
                RunEvent::PermissionRequest {
                    id: "t".into(),
                    name: "write_file".into(),
                    summary: "file".into(),
                },
                SseEvent::PermissionRequest {
                    id: "t".into(),
                    name: "write_file".into(),
                    summary: "file".into(),
                },
            ),
            (
                RunEvent::ToolResult {
                    id: "t".into(),
                    name: "write_file".into(),
                    ok: false,
                    summary: "denied".into(),
                    diff: Some(FileDiff {
                        path: "file".into(),
                        old: None,
                        new: "text".into(),
                        old_unavailable: false,
                        backup_path: None,
                    }),
                },
                SseEvent::ToolResult {
                    id: "t".into(),
                    name: "write_file".into(),
                    ok: false,
                    summary: "denied".into(),
                    diff: Some(FileDiff {
                        path: "file".into(),
                        old: None,
                        new: "text".into(),
                        old_unavailable: false,
                        backup_path: None,
                    }),
                },
            ),
            (
                RunEvent::Telemetry {
                    usage: TurnTelemetry {
                        prompt_tokens: 3,
                        completion_tokens: 4,
                        estimated: true,
                        eval_duration_ms: 99,
                    },
                },
                SseEvent::Telemetry(TurnTelemetry {
                    prompt_tokens: 3,
                    completion_tokens: 4,
                    estimated: true,
                    eval_duration_ms: 99,
                }),
            ),
            (RunEvent::Cancelled, SseEvent::Cancelled),
            (
                RunEvent::Error {
                    message: "error".into(),
                },
                SseEvent::Error("error".into()),
            ),
        ];
        for (event, expected) in pairs {
            assert_eq!(SseEvent::from(event), expected);
        }
    }
}
