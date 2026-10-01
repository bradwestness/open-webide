//! Wire protocol messages and domain types for the WebSocket process execution bridge.
//!
//! Shared across the native bridge daemon, the agent executor, and the frontend terminal.

use crate::{ChatRequest, EditorContext, RunEvent, RunSnapshot, ToolStreamChunk};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Message sent from the client (IDE frontend or agent executor) to the bridge daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeClientMessage {
    /// Hello handshake with token
    Hello {
        token: String,
    },
    RunStart {
        run_id: String,
        session_id: i64,
        content: String,
        model: Option<String>,
        editor_context: Option<EditorContext>,
    },
    RunAttach {
        run_id: String,
        last_seq: Option<u64>,
    },
    RunCancel {
        run_id: String,
    },
    RunPermission {
        run_id: String,
        tool_call_id: String,
        approved: bool,
    },
    RunList {
        session_id: i64,
    },
    CompletionStart {
        id: String,
        request: ChatRequest,
    },
    CompletionCancel {
        id: String,
    },
    /// Spawn an interactive PTY session or non-interactive process.
    Spawn {
        id: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        pty: bool,
        #[serde(default = "default_cols")]
        cols: u16,
        #[serde(default = "default_rows")]
        rows: u16,
    },
    /// Send input data to process stdin or PTY.
    Input {
        id: String,
        data: String,
    },
    /// Resize PTY terminal window dimensions.
    Resize {
        id: String,
        cols: u16,
        rows: u16,
    },
    /// Terminate process or send signal.
    Kill {
        id: String,
        #[serde(default)]
        signal: Option<String>,
    },
    /// Attach or reconnect to an active session, requesting output replay after `last_seq`.
    Attach {
        id: String,
        #[serde(default)]
        last_seq: u64,
    },
    /// List currently active sessions on the daemon.
    List,
}

fn default_cols() -> u16 {
    120
}

fn default_rows() -> u16 {
    30
}

/// Message sent from the bridge daemon to the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeServerMessage {
    /// Hello handshake success.
    HelloOk {
        user_id: Option<i64>,
        protocol: u32,
        runs: bool,
    },
    /// Hello handshake error.
    HelloError {
        message: String,
    },
    RunEvent {
        run_id: String,
        seq: u64,
        event: RunEvent,
    },
    RunSnapshot {
        run_id: String,
        session_id: i64,
        seq: u64,
        snapshot: RunSnapshot,
    },
    RunRejected {
        run_id: String,
        code: RunRejectCode,
        message: String,
    },
    Runs {
        session_id: i64,
        runs: Vec<RunInfo>,
    },
    CompletionChunk {
        id: String,
        chunk: ToolStreamChunk,
    },
    CompletionEnd {
        id: String,
        error: Option<String>,
    },
    /// Process was successfully spawned.
    Spawned {
        id: String,
        pid: u32,
        pty: bool,
    },
    /// Incremental output chunk from process stdout/stderr or PTY.
    Output {
        id: String,
        seq: u64,
        stream: String,
        data: String,
    },
    /// Process has terminated.
    Exited {
        id: String,
        #[serde(default)]
        exit_code: Option<i32>,
        #[serde(default)]
        signal: Option<String>,
    },
    /// Error notification for a session or general operation.
    Error {
        id: String,
        message: String,
    },
    /// Active sessions response.
    Sessions {
        sessions: Vec<BridgeSessionInfo>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunRejectCode {
    Unauthorized,
    Busy,
    ProjectUnavailable,
    PlanFailed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInfo {
    pub run_id: String,
    pub session_id: i64,
    pub running: bool,
    pub seq: u64,
    pub started_at: u64,
}

/// Summary info for an active or recently terminated process session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSessionInfo {
    pub id: String,
    pub command: String,
    pub running: bool,
    pub pty: bool,
    pub started_at: u64,
}

/// Headless execution result of a non-interactive shell command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CommandOutcome {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutcome {
    /// Return true if process exited with code 0.
    pub fn is_success(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// Format outcome into markdown for consumption by the LLM agent.
    pub fn to_model_text(&self) -> String {
        let mut out = String::new();
        match self.exit_code {
            Some(code) => out.push_str(&format!("Exit code: {code}\n")),
            None => out.push_str("Exit code: terminated by signal\n"),
        }
        if !self.stdout.is_empty() {
            out.push_str("\nStdout:\n");
            out.push_str(&self.stdout);
            if !self.stdout.ends_with('\n') {
                out.push('\n');
            }
        }
        if !self.stderr.is_empty() {
            out.push_str("\nStderr:\n");
            out.push_str(&self.stderr);
            if !self.stderr.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatMessage, RunItem, TurnTelemetry};

    #[test]
    fn snapshot_telemetry_tracks_message_order_for_equal_turn_usage() {
        let usage = TurnTelemetry {
            prompt_tokens: 10,
            completion_tokens: 3,
            ..Default::default()
        };
        let mut snapshot = RunSnapshot::default();
        let mut first = message();
        first.id = 7;
        snapshot.apply(&RunEvent::Message { message: first });
        snapshot.apply(&RunEvent::Telemetry { usage });
        assert_eq!(snapshot.telemetry_after_message_id, Some(7));
        let mut interim = message();
        interim.id = 8;
        interim.usage = Some(usage);
        snapshot.apply(&RunEvent::Interim { message: interim });
        assert_eq!(snapshot.telemetry_after_message_id, Some(7));
        snapshot.apply(&RunEvent::Telemetry { usage });
        assert_eq!(snapshot.telemetry_after_message_id, Some(8));
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(
            serde_json::from_str::<RunSnapshot>(&json).unwrap(),
            snapshot
        );
    }

    #[test]
    fn test_bridge_messages_json_roundtrip() {
        let hello = BridgeClientMessage::Hello {
            token: "xyz123".into(),
        };
        let json_str = serde_json::to_string(&hello).unwrap();
        assert!(json_str.contains("\"type\":\"hello\""));
        let parsed: BridgeClientMessage = serde_json::from_str(&json_str).unwrap();
        assert_eq!(hello, parsed);

        let hello_ok = BridgeServerMessage::HelloOk {
            user_id: Some(42),
            protocol: 1,
            runs: true,
        };
        let json_str = serde_json::to_string(&hello_ok).unwrap();
        assert!(json_str.contains("\"type\":\"hello_ok\""));
        let parsed: BridgeServerMessage = serde_json::from_str(&json_str).unwrap();
        assert_eq!(hello_ok, parsed);

        let hello_err = BridgeServerMessage::HelloError {
            message: "token expired".into(),
        };
        let json_str = serde_json::to_string(&hello_err).unwrap();
        assert!(json_str.contains("\"type\":\"hello_error\""));
        let parsed: BridgeServerMessage = serde_json::from_str(&json_str).unwrap();
        assert_eq!(hello_err, parsed);

        let spawn = BridgeClientMessage::Spawn {
            id: "s-1".into(),
            command: "cargo".into(),
            args: vec!["test".into()],
            cwd: Some("crates/auth".into()),
            env: HashMap::new(),
            pty: true,
            cols: 80,
            rows: 24,
        };
        let json_str = serde_json::to_string(&spawn).unwrap();
        assert!(json_str.contains("\"type\":\"spawn\""));
        let parsed: BridgeClientMessage = serde_json::from_str(&json_str).unwrap();
        assert_eq!(spawn, parsed);

        let output = BridgeServerMessage::Output {
            id: "s-1".into(),
            seq: 42,
            stream: "pty".into(),
            data: "running 1 test\n".into(),
        };
        let json_str = serde_json::to_string(&output).unwrap();
        assert!(json_str.contains("\"type\":\"output\""));
        let parsed: BridgeServerMessage = serde_json::from_str(&json_str).unwrap();
        assert_eq!(output, parsed);
    }

    fn message() -> ChatMessage {
        ChatMessage {
            id: 1,
            session_id: 2,
            role: crate::Role::Assistant,
            content: "reply".into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        }
    }

    fn events() -> Vec<(&'static str, RunEvent)> {
        vec![
            ("message", RunEvent::Message { message: message() }),
            (
                "delta",
                RunEvent::Delta {
                    content: "a".into(),
                },
            ),
            ("interim", RunEvent::Interim { message: message() }),
            (
                "tool_call",
                RunEvent::ToolCall {
                    id: "t".into(),
                    name: "read".into(),
                    summary: "read file".into(),
                },
            ),
            (
                "permission_request",
                RunEvent::PermissionRequest {
                    id: "t".into(),
                    name: "read".into(),
                    summary: "read file".into(),
                    diff: None,
                    note: None,
                },
            ),
            (
                "tool_result",
                RunEvent::ToolResult {
                    id: "t".into(),
                    name: "read".into(),
                    ok: true,
                    summary: "read".into(),
                    diff: None,
                },
            ),
            (
                "telemetry",
                RunEvent::Telemetry {
                    usage: TurnTelemetry::default(),
                },
            ),
            ("done", RunEvent::Done { message: message() }),
            ("cancelled", RunEvent::Cancelled),
            (
                "error",
                RunEvent::Error {
                    message: "failed".into(),
                },
            ),
        ]
    }

    #[test]
    fn run_protocol_roundtrips_with_wire_tags() {
        let request = ChatRequest {
            connection_id: 1,
            system_prompt: None,
            model: None,
            messages: vec![],
            tools: vec![],
        };
        let clients = vec![
            (
                "run_start",
                BridgeClientMessage::RunStart {
                    run_id: "r".into(),
                    session_id: 2,
                    content: "hello".into(),
                    model: None,
                    editor_context: None,
                },
            ),
            (
                "run_attach",
                BridgeClientMessage::RunAttach {
                    run_id: "r".into(),
                    last_seq: Some(3),
                },
            ),
            (
                "run_cancel",
                BridgeClientMessage::RunCancel { run_id: "r".into() },
            ),
            (
                "run_permission",
                BridgeClientMessage::RunPermission {
                    run_id: "r".into(),
                    tool_call_id: "t".into(),
                    approved: true,
                },
            ),
            ("run_list", BridgeClientMessage::RunList { session_id: 2 }),
            (
                "completion_start",
                BridgeClientMessage::CompletionStart {
                    id: "c".into(),
                    request,
                },
            ),
            (
                "completion_cancel",
                BridgeClientMessage::CompletionCancel { id: "c".into() },
            ),
        ];
        for (tag, message) in clients {
            let json = serde_json::to_value(&message).unwrap();
            assert_eq!(json["type"], tag);
            assert_eq!(
                serde_json::from_value::<BridgeClientMessage>(json).unwrap(),
                message
            );
        }
        let mut servers = vec![
            (
                "run_event",
                BridgeServerMessage::RunEvent {
                    run_id: "r".into(),
                    seq: 1,
                    event: RunEvent::Cancelled,
                },
            ),
            (
                "run_snapshot",
                BridgeServerMessage::RunSnapshot {
                    run_id: "r".into(),
                    session_id: 2,
                    seq: 1,
                    snapshot: RunSnapshot::default(),
                },
            ),
            (
                "runs",
                BridgeServerMessage::Runs {
                    session_id: 2,
                    runs: vec![RunInfo {
                        run_id: "r".into(),
                        session_id: 2,
                        running: true,
                        seq: 1,
                        started_at: 0,
                    }],
                },
            ),
            (
                "completion_chunk",
                BridgeServerMessage::CompletionChunk {
                    id: "c".into(),
                    chunk: ToolStreamChunk::Delta("hello".into()),
                },
            ),
            (
                "completion_end",
                BridgeServerMessage::CompletionEnd {
                    id: "c".into(),
                    error: None,
                },
            ),
        ];
        for (tag, code) in [
            ("unauthorized", RunRejectCode::Unauthorized),
            ("busy", RunRejectCode::Busy),
            ("project_unavailable", RunRejectCode::ProjectUnavailable),
            ("plan_failed", RunRejectCode::PlanFailed),
            ("unavailable", RunRejectCode::Unavailable),
        ] {
            assert_eq!(serde_json::to_value(&code).unwrap(), tag);
            servers.push((
                "run_rejected",
                BridgeServerMessage::RunRejected {
                    run_id: "r".into(),
                    code,
                    message: "rejected".into(),
                },
            ));
        }
        for (tag, message) in servers {
            let json = serde_json::to_value(&message).unwrap();
            assert_eq!(json["type"], tag);
            assert_eq!(
                serde_json::from_value::<BridgeServerMessage>(json).unwrap(),
                message
            );
        }
        for (tag, event) in events() {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["kind"], tag);
            assert_eq!(event.kind_str(), tag);
            assert_eq!(serde_json::from_value::<RunEvent>(json).unwrap(), event);
        }
    }

    #[test]
    fn snapshot_reduces_order_steps_text_telemetry_and_finish() {
        let mut snapshot = RunSnapshot::default();
        snapshot.apply(&RunEvent::Message { message: message() });
        snapshot.apply(&RunEvent::Delta {
            content: "a".into(),
        });
        snapshot.apply(&RunEvent::Delta {
            content: "b".into(),
        });
        assert_eq!(snapshot.text, "ab");
        snapshot.apply(&RunEvent::Message { message: message() });
        assert_eq!(snapshot.text, "ab");
        snapshot.apply(&RunEvent::Interim { message: message() });
        assert!(snapshot.text.is_empty());
        snapshot.apply(&RunEvent::PermissionRequest {
            id: "t".into(),
            name: "read".into(),
            summary: "file".into(),
            diff: None,
            note: None,
        });
        assert!(matches!(&snapshot.items[3], RunItem::Step(step) if step.awaiting_permission));
        snapshot.apply(&RunEvent::Delta {
            content: "c".into(),
        });
        snapshot.apply(&RunEvent::ToolCall {
            id: "t".into(),
            name: "read".into(),
            summary: "file".into(),
        });
        assert!(snapshot.text.is_empty());
        assert_eq!(snapshot.items.len(), 4);
        assert!(matches!(&snapshot.items[3], RunItem::Step(step) if !step.awaiting_permission));
        snapshot.apply(&RunEvent::ToolResult {
            id: "t".into(),
            name: "read".into(),
            ok: false,
            summary: "failed".into(),
            diff: None,
        });
        assert!(
            matches!(&snapshot.items[3], RunItem::Step(step) if step.result.as_ref().is_some_and(|result| !result.ok))
        );
        snapshot.apply(&RunEvent::ToolResult {
            id: "other".into(),
            name: "write".into(),
            ok: true,
            summary: "created".into(),
            diff: None,
        });
        assert_eq!(snapshot.items.len(), 5);
        assert!(
            snapshot.items[..3]
                .iter()
                .all(|item| matches!(item, RunItem::Message(_)))
        );
        let usage = TurnTelemetry {
            prompt_tokens: 10,
            ..Default::default()
        };
        snapshot.apply(&RunEvent::Telemetry { usage });
        assert_eq!(snapshot.telemetry, Some(usage));
        for event in [
            RunEvent::Done { message: message() },
            RunEvent::Cancelled,
            RunEvent::Error {
                message: "error".into(),
            },
        ] {
            snapshot.apply(&event);
            assert_eq!(snapshot.finished, Some(event));
        }
        let json = serde_json::to_string(&snapshot).unwrap();
        assert_eq!(
            serde_json::from_str::<RunSnapshot>(&json).unwrap(),
            snapshot
        );
    }

    #[test]
    fn test_command_outcome_formatting() {
        let outcome = CommandOutcome {
            exit_code: Some(101),
            stdout: "running 2 tests\ntest foo ... ok\n".into(),
            stderr: "error: test failed\n".into(),
        };
        let text = outcome.to_model_text();
        assert!(text.contains("Exit code: 101"));
        assert!(text.contains("Stdout:\nrunning 2 tests"));
        assert!(text.contains("Stderr:\nerror: test failed"));
    }
}
