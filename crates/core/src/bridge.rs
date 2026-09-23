//! Wire protocol messages and domain types for the WebSocket process execution bridge.
//!
//! Shared across the native bridge daemon, the agent executor, and the frontend terminal.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Message sent from the client (IDE frontend or agent executor) to the bridge daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeClientMessage {
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
    Input { id: String, data: String },
    /// Resize PTY terminal window dimensions.
    Resize { id: String, cols: u16, rows: u16 },
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
    /// Process was successfully spawned.
    Spawned { id: String, pid: u32, pty: bool },
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
    Error { id: String, message: String },
    /// Active sessions response.
    Sessions { sessions: Vec<BridgeSessionInfo> },
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

    #[test]
    fn test_bridge_messages_json_roundtrip() {
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
