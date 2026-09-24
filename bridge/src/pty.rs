//! Interactive pseudo-terminal (PTY) process spawning via portable-pty.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::mpsc;

use crate::proc::Signal;
use crate::session::Session;

/// Spawn an interactive PTY session.
#[allow(clippy::too_many_arguments)] // bundling into a config struct is an API change for callers, not a lint fix
pub fn spawn_pty(
    id: String,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: HashMap<String, String>,
    cols: u16,
    rows: u16,
    workspace_root: &Path,
) -> Result<Arc<Session>, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty failed: {e}"))?;

    // Determine working directory: confined to workspace_root
    let effective_cwd = crate::paths::resolve_in_root(workspace_root, cwd.as_deref())?;

    let mut cmd = CommandBuilder::new(&command);
    cmd.args(&args);
    cmd.cwd(&effective_cwd);

    for (k, v) in env {
        cmd.env(k, v);
    }

    // Spawn child in slave PTY
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("failed to spawn '{command}': {e}"))?;
    drop(pair.slave);

    // Children are session leaders (portable-pty calls `setsid`), so their pid is also their
    // pgid.
    let pid = child.process_id().map(|id| id as i32);

    let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(128);
    let (resize_tx, mut resize_rx) = mpsc::channel::<(u16, u16)>(32);
    let (kill_tx, mut kill_rx) = mpsc::channel::<Signal>(4);

    let display_cmd = if args.is_empty() {
        command.clone()
    } else {
        format!("{command} {}", args.join(" "))
    };

    let session = Arc::new(Session::new(
        id.clone(),
        display_cmd,
        true,
        pid,
        stdin_tx.clone(),
        resize_tx,
        kill_tx,
    ));

    // 1. Thread for reading PTY master output
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("clone PTY reader failed: {e}"))?;
    let sess_clone = session.clone();

    std::thread::Builder::new()
        .name(format!("pty-read-{id}"))
        .spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]).to_string();
                        sess_clone.emit_output("pty", &text);
                    }
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::Interrupted {
                            break;
                        }
                    }
                }
            }
        })
        .map_err(|e| format!("spawn read thread failed: {e}"))?;

    // 2. Thread for writing to PTY master stdin
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("take PTY writer failed: {e}"))?;

    std::thread::Builder::new()
        .name(format!("pty-write-{id}"))
        .spawn(move || {
            while let Some(data) = stdin_rx.blocking_recv() {
                if writer.write_all(data.as_bytes()).is_err() || writer.flush().is_err() {
                    break;
                }
            }
        })
        .map_err(|e| format!("spawn write thread failed: {e}"))?;

    // 3. Task for resizing PTY
    let master = Arc::new(std::sync::Mutex::new(pair.master));
    let master_resize = master.clone();
    tokio::spawn(async move {
        while let Some((c, r)) = resize_rx.recv().await {
            if let Ok(m) = master_resize.lock() {
                let _ = m.resize(PtySize {
                    rows: r,
                    cols: c,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
    });

    // 4. Thread for handling kill requests. `Int` writes ^C so the foreground job's own signal
    // handling fires, matching a real terminal's Ctrl+C; `Term`/`Hup`/`Kill` signal the PTY's
    // process group directly, falling back to the portable-pty killer (a direct kill of the
    // immediate child only) when the pid is unavailable.
    let mut killer = child.clone_killer();
    std::thread::Builder::new()
        .name(format!("pty-kill-{id}"))
        .spawn(move || {
            while let Some(sig) = kill_rx.blocking_recv() {
                match sig {
                    Signal::Int => {
                        let _ = stdin_tx.blocking_send("\x03".to_string());
                    }
                    #[cfg(unix)]
                    Signal::Term | Signal::Hup | Signal::Kill => match pid {
                        Some(pgid) => crate::proc::signal_group(pgid, sig),
                        None => {
                            let _ = killer.kill();
                        }
                    },
                    #[cfg(windows)]
                    Signal::Term | Signal::Hup | Signal::Kill => {
                        let _ = killer.kill();
                    }
                }
            }
        })
        .map_err(|e| format!("spawn kill thread failed: {e}"))?;

    // 5. Thread for supervising / waiting child exit
    let sess_exit = session.clone();
    std::thread::Builder::new()
        .name(format!("pty-wait-{id}"))
        .spawn(move || {
            let mut child = child;
            // Wait for child process exit
            match child.wait() {
                Ok(status) => {
                    let code = if status.success() {
                        Some(0)
                    } else {
                        // ExitCode in portable-pty
                        Some(1)
                    };
                    sess_exit.emit_exit(code, None);
                }
                Err(e) => {
                    sess_exit.emit_error(&format!("wait failed: {e}"));
                    sess_exit.emit_exit(Some(1), None);
                }
            }
        })
        .map_err(|e| format!("spawn wait thread failed: {e}"))?;

    Ok(session)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Sending `Signal::Int` writes `^C` through the PTY's stdin, which the tty's line
    /// discipline turns into a real SIGINT to the foreground job — here, the shell's own trap,
    /// since it isn't running a separate foregrounded child. The exit code that trap produces is
    /// asserted in step 26; this only checks the interrupt reached it at all.
    #[tokio::test]
    async fn pty_sigint_interrupts_foreground() {
        let temp_dir = crate::paths::canonical_root(&std::env::temp_dir()).unwrap();
        let session = spawn_pty(
            "sess-pty-int".into(),
            "sh".into(),
            vec![],
            None,
            Default::default(),
            80,
            24,
            &temp_dir,
        )
        .unwrap();

        let mut rx = session.broadcast_tx.subscribe();
        tokio::time::sleep(Duration::from_millis(200)).await;
        // Job control would put `sleep` in its own process group, so the tty's ^C would reach
        // only that group and never the shell's own trap; disable it so the whole foreground
        // pipeline stays in the shell's pgid, matching a non-interactive job-less script.
        session.send_input("set +m\n".to_string()).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        session
            .send_input("trap 'echo got-int' INT\n".to_string())
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        session.send_input("sleep 5\n".to_string()).await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        session.kill(Signal::Int).await;

        let saw_got_int = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(openwebide_core::BridgeServerMessage::Output { data, .. }) =
                    rx.recv().await
                    && data.contains("got-int")
                {
                    return;
                }
            }
        })
        .await;
        assert!(
            saw_got_int.is_ok(),
            "expected the shell's SIGINT trap to fire"
        );
    }
}
