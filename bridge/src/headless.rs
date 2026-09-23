//! Headless (non-PTY) command execution with bounded timeouts and captured stdout/stderr.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use openwebide_core::CommandOutcome;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::session::Session;

/// Spawn a non-interactive process attached to a Session.
pub fn spawn_headless(
    id: String,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: HashMap<String, String>,
    workspace_root: &Path,
) -> Result<Arc<Session>, String> {
    let effective_cwd = match cwd {
        Some(ref rel) if !rel.is_empty() => {
            let candidate = workspace_root.join(rel);
            if !candidate.starts_with(workspace_root) {
                return Err(format!("cwd escapes workspace root: {rel}"));
            }
            candidate
        }
        _ => workspace_root.to_path_buf(),
    };

    let mut cmd = Command::new(&command);
    cmd.args(&args);
    cmd.current_dir(&effective_cwd);
    cmd.envs(env);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn '{command}': {e}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();

    let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(128);
    let (resize_tx, _resize_rx) = mpsc::channel::<(u16, u16)>(1);
    let (kill_tx, mut kill_rx) = mpsc::channel::<Option<String>>(4);

    let display_cmd = if args.is_empty() {
        command.clone()
    } else {
        format!("{command} {}", args.join(" "))
    };

    let session = Arc::new(Session::new(
        id,
        display_cmd,
        false,
        stdin_tx,
        resize_tx,
        kill_tx,
    ));

    // 1. Stdout reader task
    if let Some(stdout) = stdout {
        let sess = session.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                sess.emit_output("stdout", &format!("{line}\n"));
            }
        });
    }

    // 2. Stderr reader task
    if let Some(stderr) = stderr {
        let sess = session.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                sess.emit_output("stderr", &format!("{line}\n"));
            }
        });
    }

    // 3. Stdin writer task
    if let Some(mut stdin) = stdin {
        tokio::spawn(async move {
            while let Some(data) = stdin_rx.recv().await {
                if stdin.write_all(data.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });
    }

    // 4. Process supervisor task
    let sess_exit = session.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = kill_rx.recv() => {
                let _ = child.kill().await;
                sess_exit.emit_exit(None, Some("SIGKILL".into()));
            }
            status = child.wait() => {
                match status {
                    Ok(s) => sess_exit.emit_exit(s.code(), None),
                    Err(e) => {
                        sess_exit.emit_error(&format!("wait failed: {e}"));
                        sess_exit.emit_exit(Some(1), None);
                    }
                }
            }
        }
    });

    Ok(session)
}

/// Execute a shell command directly with a timeout, capturing stdout and stderr into a [`CommandOutcome`].
pub async fn execute_command_direct(
    command_str: &str,
    workspace_root: &Path,
    timeout_secs: u64,
) -> Result<CommandOutcome, String> {
    // Run command via default shell (sh -c on Unix, cmd /C on Windows)
    #[cfg(unix)]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command_str);
        c
    };

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command_str);
        c
    };

    cmd.current_dir(workspace_root);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn command '{command_str}': {e}"))?;

    let timeout_duration = Duration::from_secs(timeout_secs.clamp(1, 300));
    match timeout(timeout_duration, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Ok(CommandOutcome {
                exit_code: output.status.code(),
                stdout,
                stderr,
            })
        }
        Ok(Err(e)) => Err(format!("execution error: {e}")),
        Err(_) => Err(format!("command timed out after {timeout_secs}s")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execute_command_direct_success() {
        let temp_dir = std::env::temp_dir();
        let res = execute_command_direct("echo 'openwebide'", &temp_dir, 5).await;
        assert!(res.is_ok());
        let outcome = res.unwrap();
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.stdout.contains("openwebide"));
        assert!(outcome.is_success());
    }

    #[tokio::test]
    async fn test_execute_command_direct_failure() {
        let temp_dir = std::env::temp_dir();
        let res = execute_command_direct("exit 42", &temp_dir, 5).await;
        assert!(res.is_ok());
        let outcome = res.unwrap();
        assert_eq!(outcome.exit_code, Some(42));
        assert!(!outcome.is_success());
    }

    #[tokio::test]
    async fn test_execute_command_direct_timeout() {
        let temp_dir = std::env::temp_dir();
        let res = execute_command_direct("sleep 3", &temp_dir, 1).await;
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("timed out"));
    }
}
