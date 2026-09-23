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
    /// Telemetry metrics for the turn.
    Telemetry(TurnTelemetry),
    /// The run was cancelled; the stream ends after this.
    Cancelled,
    /// A provider or persistence error; the stream ends after this.
    Error(String),
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
