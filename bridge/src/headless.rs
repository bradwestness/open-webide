//! Headless (non-PTY) command execution with bounded timeouts and captured stdout/stderr.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use openwebide_core::CommandOutcome;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::proc::Signal;
use crate::session::Session;

/// Head bytes kept verbatim per captured stream before the omission marker.
const CAPTURE_HEAD_BYTES: usize = 256 * 1024;
/// Tail bytes kept verbatim per captured stream after the omission marker.
const CAPTURE_TAIL_BYTES: usize = 768 * 1024;

/// Accumulates a byte stream, keeping the first `head_cap` bytes and the last `tail_cap` bytes,
/// with everything in between dropped and reported via an omission marker. Bounds memory use for
/// commands that produce far more output than anyone will read.
struct CappedCapture {
    head: Vec<u8>,
    head_cap: usize,
    tail: VecDeque<u8>,
    tail_cap: usize,
    total: usize,
}

impl CappedCapture {
    fn new(head_cap: usize, tail_cap: usize) -> Self {
        Self {
            head: Vec::new(),
            head_cap,
            tail: VecDeque::new(),
            tail_cap,
            total: 0,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.total += chunk.len();
        let remaining_head = self.head_cap.saturating_sub(self.head.len());
        let (to_head, to_tail) = if chunk.len() <= remaining_head {
            (chunk, &[][..])
        } else {
            chunk.split_at(remaining_head)
        };
        self.head.extend_from_slice(to_head);
        self.tail.extend(to_tail);
        while self.tail.len() > self.tail_cap {
            self.tail.pop_front();
        }
    }

    fn finish(self) -> String {
        let kept = self.head.len() + self.tail.len();
        let tail_bytes: Vec<u8> = self.tail.into_iter().collect();
        if self.total <= kept {
            let mut bytes = self.head;
            bytes.extend(tail_bytes);
            return String::from_utf8_lossy(&bytes).into_owned();
        }
        let omitted = self.total - kept;
        let mut out = String::from_utf8_lossy(&self.head).into_owned();
        out.push_str(&format!("\n[… {omitted} bytes omitted …]\n"));
        out.push_str(&String::from_utf8_lossy(&tail_bytes));
        out
    }
}

/// Read `reader` to EOF, capping the captured content, and return the resulting text.
async fn capture_stream<R: AsyncRead + Unpin>(mut reader: R) -> String {
    let mut capture = CappedCapture::new(CAPTURE_HEAD_BYTES, CAPTURE_TAIL_BYTES);
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => capture.push(&buf[..n]),
        }
    }
    capture.finish()
}

/// Spawn a non-interactive process attached to a Session.
pub fn spawn_headless(
    id: String,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: HashMap<String, String>,
    workspace_root: &Path,
) -> Result<Arc<Session>, String> {
    let effective_cwd = crate::paths::resolve_in_root(workspace_root, cwd.as_deref())?;

    let mut cmd = Command::new(&command);
    cmd.args(&args);
    cmd.current_dir(&effective_cwd);
    cmd.envs(env);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn '{command}': {e}"))?;

    let pid = child.id().map(|id| id as i32);

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();

    let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(128);
    let (resize_tx, _resize_rx) = mpsc::channel::<(u16, u16)>(1);
    let (kill_tx, mut kill_rx) = mpsc::channel::<Signal>(4);

    let display_cmd = if args.is_empty() {
        command.clone()
    } else {
        format!("{command} {}", args.join(" "))
    };

    let session = Arc::new(Session::new(
        id,
        display_cmd,
        false,
        pid,
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

    // 4. Process supervisor task: signals the group on each `Kill` message (and keeps waiting
    // for exit afterward, so escalating from SIGTERM to SIGKILL works), and reports the exact
    // exit code/signal once the child is reaped.
    let sess_exit = session.clone();
    tokio::spawn(async move {
        let mut kill_channel_open = true;

        // `child.wait()` is cancel-safe, so a fresh one each iteration (rather than a single
        // pinned future reused across iterations) is fine, and it keeps `child` free between
        // iterations for the Windows kill branch to call `start_kill()` on.
        loop {
            tokio::select! {
                sig = kill_rx.recv(), if kill_channel_open => {
                    match sig {
                        Some(sig) => {
                            #[cfg(unix)]
                            if let Some(pgid) = pid {
                                crate::proc::signal_group(pgid, sig);
                            }
                            #[cfg(windows)]
                            {
                                let _ = sig;
                                let _ = child.start_kill();
                            }
                        }
                        None => kill_channel_open = false,
                    }
                }
                status = child.wait() => {
                    match status {
                        Ok(s) => {
                            #[cfg(unix)]
                            let signal = {
                                use std::os::unix::process::ExitStatusExt;
                                s.signal().map(crate::proc::signal_name)
                            };
                            #[cfg(windows)]
                            let signal = None;
                            sess_exit.emit_exit(s.code(), signal);
                        }
                        Err(e) => {
                            sess_exit.emit_error(&format!("wait failed: {e}"));
                            sess_exit.emit_exit(Some(1), None);
                        }
                    }
                    break;
                }
            }
        }
    });

    Ok(session)
}

/// Execute a shell command directly with a timeout, capturing stdout and stderr into a
/// [`CommandOutcome`]. `disconnected` resolves when the caller no longer needs the result (e.g.
/// the run driving this call was cancelled); the command's process group is killed either way.
/// The HTTP `/exec` handler passes [`std::future::pending`] here: hyper simply drops this whole
/// future when its client disconnects mid-request, and a `GroupGuard` dropped along with it is
/// what kills the process group in that case.
pub async fn execute_command_direct(
    command_str: &str,
    cwd: &Path,
    timeout_secs: u64,
    disconnected: impl std::future::Future<Output = ()>,
) -> Result<CommandOutcome, String> {
    // Run command via default shell (sh -c on Unix, cmd /C on Windows)
    #[cfg(unix)]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command_str);
        c.process_group(0);
        c
    };

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command_str);
        c
    };

    cmd.current_dir(cwd);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn command '{command_str}': {e}"))?;

    let pid = child.id().map(|id| id as i32);
    // Armed for as long as this future can still be dropped mid-flight (e.g. hyper dropping the
    // `/exec` handler on a disconnected client) instead of running to a controlled exit —
    // including while the timeout/cancel branches below are themselves awaiting
    // `terminate_group`'s grace period. Disarmed once a controlled exit is reached, so a
    // successful command doesn't have its intentionally backgrounded/detached jobs killed.
    #[cfg(unix)]
    let group_guard = pid.map(crate::proc::GroupGuard::new);

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let out_task = tokio::spawn(capture_stream(stdout));
    let err_task = tokio::spawn(capture_stream(stderr));

    let timeout_duration = Duration::from_secs(timeout_secs.clamp(1, 300));

    // Waits for the leader AND drains both pipes as one future, so the timeout/cancel branches
    // below actually race it: a backgrounded job that outlives the leader but still holds a pipe
    // open (e.g. `sleep 20 &`) would otherwise hang `out_task`/`err_task` forever after `child`
    // exits, with neither the timer nor `disconnected` polled anymore.
    let run = async {
        let status = child.wait().await;
        let stdout = out_task.await.unwrap_or_default();
        let stderr = err_task.await.unwrap_or_default();
        (status, stdout, stderr)
    };

    let result = tokio::select! {
        (status, stdout, stderr) = run => {
            match status {
                Ok(status) => Ok(CommandOutcome {
                    exit_code: status.code(),
                    stdout,
                    stderr,
                }),
                Err(e) => Err(format!("execution error: {e}")),
            }
        }
        () = tokio::time::sleep(timeout_duration) => {
            // Reap the leader concurrently with signalling it: otherwise it sits as an unreaped
            // zombie, `terminate_group`'s `kill(-pgid, 0)` liveness poll keeps seeing it, and
            // every timeout pays the full grace period even when nothing else is left.
            // `terminate_spawned_group` is a no-op on Windows (no process groups), so kill the
            // direct child first there or `child.wait()` would just wait the command out.
            #[cfg(windows)]
            let _ = child.start_kill();
            let _ = tokio::join!(terminate_spawned_group(pid), child.wait());
            Err(format!("command timed out after {timeout_secs}s"))
        }
        () = disconnected => {
            #[cfg(windows)]
            let _ = child.start_kill();
            let _ = tokio::join!(terminate_spawned_group(pid), child.wait());
            Err("command cancelled".to_string())
        }
    };

    #[cfg(unix)]
    if let Some(guard) = group_guard {
        guard.disarm();
    }

    result
}

#[cfg(unix)]
async fn terminate_spawned_group(pid: Option<i32>) {
    if let Some(pgid) = pid {
        crate::proc::terminate_group(pgid, Duration::from_secs(2)).await;
    }
}

#[cfg(windows)]
async fn terminate_spawned_group(_pid: Option<i32>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionManager;

    #[tokio::test]
    async fn test_execute_command_direct_success() {
        let temp_dir = std::env::temp_dir();
        let res =
            execute_command_direct("echo 'openwebide'", &temp_dir, 5, std::future::pending()).await;
        assert!(res.is_ok());
        let outcome = res.unwrap();
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.stdout.contains("openwebide"));
        assert!(outcome.is_success());
    }

    #[tokio::test]
    async fn test_execute_command_direct_failure() {
        let temp_dir = std::env::temp_dir();
        let res = execute_command_direct("exit 42", &temp_dir, 5, std::future::pending()).await;
        assert!(res.is_ok());
        let outcome = res.unwrap();
        assert_eq!(outcome.exit_code, Some(42));
        assert!(!outcome.is_success());
    }

    #[tokio::test]
    async fn test_execute_command_direct_timeout() {
        let temp_dir = std::env::temp_dir();
        let started = std::time::Instant::now();
        let res = execute_command_direct("sleep 3", &temp_dir, 1, std::future::pending()).await;
        let elapsed = started.elapsed();
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("timed out"));
        // Reaping the leader concurrently with `terminate_group`'s liveness poll means it need
        // not pay the full 2s grace period on top of the 1s timeout.
        assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
    }

    #[tokio::test]
    async fn exec_stdin_is_null() {
        let temp_dir = std::env::temp_dir();
        let started = std::time::Instant::now();
        let res = execute_command_direct("cat", &temp_dir, 5, std::future::pending()).await;
        assert!(res.is_ok(), "{res:?}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "cat should see immediate EOF on stdin, not hang"
        );
    }

    #[tokio::test]
    async fn exec_output_is_capped() {
        let temp_dir = std::env::temp_dir();
        let res = execute_command_direct(
            "yes x | head -c 5000000",
            &temp_dir,
            10,
            std::future::pending(),
        )
        .await
        .unwrap();
        assert!(
            res.stdout.len() <= CAPTURE_HEAD_BYTES + CAPTURE_TAIL_BYTES + 200,
            "capped output grew to {} bytes",
            res.stdout.len()
        );
        assert!(res.stdout.contains("bytes omitted"));
    }

    /// A backgrounded job that outlives the shell but still holds stdout open must not defeat
    /// the timeout: the leader (`sh`) exits well before the 2s timeout, but the pipe stays open
    /// until the backgrounded `sleep 20` exits, so a naive "wait for EOF" drain would hang the
    /// whole call regardless of the timeout.
    #[cfg(unix)]
    #[tokio::test]
    async fn exec_timeout_fires_despite_backgrounded_stdout_holder() {
        let temp_dir = std::env::temp_dir();
        let started = std::time::Instant::now();
        let res = execute_command_direct(
            "sleep 20 & echo started",
            &temp_dir,
            2,
            std::future::pending(),
        )
        .await;
        let elapsed = started.elapsed();
        assert!(
            res.unwrap_err().contains("timed out"),
            "elapsed {elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(4), "took {elapsed:?}");
    }

    /// Same as above, but for the `disconnected` cancellation path rather than the timeout.
    #[cfg(unix)]
    #[tokio::test]
    async fn exec_cancel_fires_despite_backgrounded_stdout_holder() {
        let temp_dir = std::env::temp_dir();
        let started = std::time::Instant::now();
        let res = execute_command_direct(
            "sleep 20 &",
            &temp_dir,
            300,
            tokio::time::sleep(Duration::from_secs(1)),
        )
        .await;
        let elapsed = started.elapsed();
        assert!(
            res.unwrap_err().contains("cancelled"),
            "elapsed {elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
    }

    /// A successful command must not have its intentionally detached/backgrounded jobs killed:
    /// `GroupGuard` exists for the case where this future is dropped mid-flight (a disconnected
    /// client), not for a command that ran to completion.
    #[cfg(unix)]
    #[tokio::test]
    async fn exec_success_does_not_kill_detached_job() {
        let dir = crate::paths::canonical_root(&std::env::temp_dir()).unwrap();
        let dir = dir.join(format!("owide-detached-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("pid");
        let cmd = format!(
            "nohup sleep 5 >/dev/null 2>&1 & echo $! > {}",
            pidfile.display()
        );

        let res = execute_command_direct(&cmd, &dir, 5, std::future::pending())
            .await
            .unwrap();
        assert!(res.is_success());

        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        // SAFETY: signal 0 sends nothing; it only probes whether `pid` is still alive.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        // SAFETY: pid is our own freshly-spawned detached child; terminating it in test cleanup
        // is always sound.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            alive,
            "a detached background job must survive a successful /exec"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_timeout_kills_process_group() {
        let dir = crate::paths::canonical_root(&std::env::temp_dir()).unwrap();
        let dir = dir.join(format!("owide-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m1 = dir.join("direct");
        let m2 = dir.join("grandchild");
        let cmd = format!(
            "(sleep 2; touch {}) & sleep 2; touch {}",
            m2.display(),
            m1.display()
        );
        let res = execute_command_direct(&cmd, &dir, 1, std::future::pending()).await;
        assert!(res.unwrap_err().contains("timed out"));
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(
            !m1.exists() && !m2.exists(),
            "timed-out process group must be dead"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn headless_kill_sigint_reports_signal() {
        let temp_dir = crate::paths::canonical_root(&std::env::temp_dir()).unwrap();
        let session = spawn_headless(
            "sess-int".into(),
            "sleep".into(),
            vec!["5".into()],
            None,
            Default::default(),
            &temp_dir,
        )
        .unwrap();

        let mut rx = session.broadcast_tx.subscribe();
        session.kill(Signal::Int).await;

        let exited = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(openwebide_core::BridgeServerMessage::Exited {
                    exit_code, signal, ..
                }) = rx.recv().await
                {
                    return (exit_code, signal);
                }
            }
        })
        .await
        .expect("session should exit");

        assert_eq!(exited, (None, Some("SIGINT".to_string())));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn headless_kill_reaches_grandchild() {
        let dir = crate::paths::canonical_root(&std::env::temp_dir()).unwrap();
        let dir = dir.join(format!("owide-killgroup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("grandchild");
        let cmd = format!("(sleep 5; touch {}) & wait", marker.display());

        let session = spawn_headless(
            "sess-kill".into(),
            "sh".into(),
            vec!["-c".into(), cmd],
            None,
            Default::default(),
            &dir,
        )
        .unwrap();

        session.kill(Signal::Kill).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!session.running.load(std::sync::atomic::Ordering::SeqCst));

        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(
            !marker.exists(),
            "SIGKILL to the group must reach the grandchild"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_all_terminates_running() {
        let dir = crate::paths::canonical_root(&std::env::temp_dir()).unwrap();
        let mgr = SessionManager::new();
        let session = spawn_headless(
            "sess-killall".into(),
            "sleep".into(),
            vec!["30".into()],
            None,
            Default::default(),
            &dir,
        )
        .unwrap();
        mgr.insert(session.clone());

        mgr.kill_all().await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(!session.running.load(std::sync::atomic::Ordering::SeqCst));
    }
}
