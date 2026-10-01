//! Interactive pseudo-terminal (PTY) process spawning via portable-pty.

use std::io::{Read, Write};
use std::sync::Arc;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tokio::sync::mpsc;

use crate::exec::proc::Signal;
use crate::terminals::session::Session;

/// Spawn an interactive PTY session.
pub fn spawn_pty(
    id: String,
    spec: crate::exec::SpawnSpec,
    cols: u16,
    rows: u16,
) -> Result<Arc<Session>, String> {
    let display_cmd = spec.display_cmd();
    let crate::exec::SpawnSpec {
        command,
        args,
        cwd: effective_cwd,
        env,
        ..
    } = spec;
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty failed: {e}"))?;

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
    let pid = child.process_id().and_then(|id| i32::try_from(id).ok());

    let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(128);
    let (resize_tx, mut resize_rx) = mpsc::channel::<(u16, u16)>(32);
    let (kill_tx, mut kill_rx) = mpsc::channel::<Signal>(4);

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

    let (reader_exit_tx, reader_exit_rx) = std::sync::mpsc::channel::<()>();
    std::thread::Builder::new()
        .name(format!("pty-read-{id}"))
        .spawn(move || {
            let mut buf = [0u8; 8192];
            let mut decoder = openwebide_core::utf8::Utf8Decoder::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let text = decoder.finish();
                        if !text.is_empty() {
                            sess_clone.push_output("pty", &text);
                        }
                        break;
                    }
                    Ok(n) => {
                        let text = decoder.push(&buf[..n]);
                        if !text.is_empty() {
                            sess_clone.push_output("pty", &text);
                        }
                    }
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::Interrupted {
                            let text = decoder.finish();
                            if !text.is_empty() {
                                sess_clone.push_output("pty", &text);
                            }
                            break;
                        }
                    }
                }
            }
            let _ = reader_exit_tx.send(());
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
                        Some(pgid) => crate::exec::proc::signal_group(pgid, sig),
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
                    let _ = reader_exit_rx.recv_timeout(std::time::Duration::from_millis(500));
                    sess_exit.emit_exit(
                        i32::try_from(status.exit_code()).ok(),
                        status.signal().map(ToString::to_string),
                    );
                }
                Err(e) => {
                    let _ = reader_exit_rx.recv_timeout(std::time::Duration::from_millis(500));
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
            crate::exec::SpawnSpec::in_root(
                "sh".into(),
                vec![],
                None,
                Default::default(),
                &temp_dir,
            )
            .unwrap(),
            80,
            24,
        )
        .unwrap();

        let mut rx = session.ring.lock().unwrap().subscribe();
        tokio::time::sleep(Duration::from_millis(200)).await;
        // Job control would put `sleep` in its own process group, so the tty's ^C would reach
        // only that group and never the shell's own trap; disable it so the whole foreground
        // pipeline stays in the shell's pgid, matching a non-interactive job-less script.
        let _ = session.try_send_input("set +m\n".to_string());
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = session.try_send_input("trap 'echo got-int' INT\n".to_string());
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = session.try_send_input("sleep 5\n".to_string());
        tokio::time::sleep(Duration::from_millis(200)).await;

        session.try_kill(Signal::Int);

        let saw_got_int = tokio::time::timeout(Duration::from_secs(5), async {
            let mut cursor = 0;
            loop {
                let _ = rx.changed().await;
                let (batch, _) = session.ring.lock().unwrap().read_after(cursor, 100);
                for (seq, msg) in batch {
                    cursor = seq;
                    if let openwebide_core::BridgeServerMessage::Output { data, .. } = msg
                        && data.contains("got-int")
                    {
                        return;
                    }
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
