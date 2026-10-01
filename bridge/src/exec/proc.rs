//! Process-group signal delivery and forceful termination.
//!
//! Every spawned command runs as its own process group leader (`process_group(0)`, or a PTY
//! session leader via `setsid`), so its pgid equals its pid. Signalling the negated pgid reaches
//! the whole group, including grandchildren that outlive the direct child (e.g. backgrounded
//! jobs), which a plain `child.kill()` never does.

use std::time::Duration;

/// A signal requested by a client `Kill` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Int,
    Term,
    Kill,
    Hup,
}

/// Parse a client-supplied signal name. `None` (no `signal` field) defaults to `Kill`; an
/// unrecognized name is an error the caller should report back to the client.
pub fn parse_signal(name: Option<&str>) -> Result<Signal, String> {
    let Some(name) = name else {
        return Ok(Signal::Kill);
    };
    match name.to_ascii_uppercase().as_str() {
        "SIGINT" | "INT" => Ok(Signal::Int),
        "SIGTERM" | "TERM" => Ok(Signal::Term),
        "SIGKILL" | "KILL" => Ok(Signal::Kill),
        "SIGHUP" | "HUP" => Ok(Signal::Hup),
        _ => Err("unsupported signal".to_string()),
    }
}

/// Map a raw signal number to its conventional name, e.g. for reporting how a process exited.
pub fn signal_name(sig: i32) -> String {
    match sig {
        1 => "SIGHUP".to_string(),
        2 => "SIGINT".to_string(),
        3 => "SIGQUIT".to_string(),
        9 => "SIGKILL".to_string(),
        15 => "SIGTERM".to_string(),
        other => format!("SIG{other}"),
    }
}

#[cfg(unix)]
fn to_libc_signal(s: Signal) -> libc::c_int {
    match s {
        Signal::Int => libc::SIGINT,
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
        Signal::Hup => libc::SIGHUP,
    }
}

/// Send `s` to every process in group `pgid`.
#[cfg(unix)]
pub fn signal_group(pgid: i32, s: Signal) {
    // SAFETY: killpg with a valid pgid and signal number is always sound; a stale/exited pgid
    // just yields ESRCH, which we don't need to distinguish here.
    unsafe {
        libc::killpg(pgid, to_libc_signal(s));
    }
}

/// Whether process group `pgid` still has any live member, probed with the null signal.
#[cfg(unix)]
fn group_alive(pgid: i32) -> bool {
    // SAFETY: signal 0 sends nothing; it only probes whether the target exists and is
    // signalable, which is always a sound call to make.
    let ret = unsafe { libc::kill(-pgid, 0) };
    if ret == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Terminate process group `pgid`: send SIGTERM, poll for exit until `grace` elapses, then
/// SIGKILL. Returns once the group is confirmed gone or the SIGKILL has been sent.
#[cfg(unix)]
pub async fn terminate_group(pgid: i32, grace: Duration) {
    signal_group(pgid, Signal::Term);

    let deadline = tokio::time::Instant::now() + grace;
    loop {
        if !group_alive(pgid) {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    signal_group(pgid, Signal::Kill);
}

/// Kills process group `pgid` on drop: SIGTERM immediately, then a SIGKILL follow-up after a
/// grace period. Used to clean up a command's process group when the future driving it is
/// dropped mid-flight (e.g. a disconnected HTTP client) rather than run to completion, where an
/// `.await`-based `terminate_group` call would never get a chance to run.
#[cfg(unix)]
pub struct GroupGuard(i32);

#[cfg(unix)]
impl GroupGuard {
    pub fn new(pgid: i32) -> Self {
        Self(pgid)
    }

    /// Defuse the guard: its `Drop` performs no cleanup. For use once the future it was guarding
    /// has reached a controlled exit (the command finished, or was explicitly terminated already)
    /// instead of being dropped mid-flight, so a successful command's intentionally
    /// backgrounded/detached jobs aren't killed along with it.
    pub fn disarm(self) {
        std::mem::forget(self);
    }
}

#[cfg(unix)]
impl Drop for GroupGuard {
    fn drop(&mut self) {
        let pgid = self.0;
        signal_group(pgid, Signal::Term);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            signal_group(pgid, Signal::Kill);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_signal_accepts_short_and_long_names_case_insensitively() {
        assert_eq!(parse_signal(Some("SIGINT")), Ok(Signal::Int));
        assert_eq!(parse_signal(Some("int")), Ok(Signal::Int));
        assert_eq!(parse_signal(Some("SigTerm")), Ok(Signal::Term));
        assert_eq!(parse_signal(Some("KILL")), Ok(Signal::Kill));
        assert_eq!(parse_signal(Some("sighup")), Ok(Signal::Hup));
    }

    #[test]
    fn parse_signal_none_defaults_to_kill() {
        assert_eq!(parse_signal(None), Ok(Signal::Kill));
    }

    #[test]
    fn parse_signal_rejects_unknown_names() {
        assert_eq!(
            parse_signal(Some("SIGWHATEVER")),
            Err("unsupported signal".to_string())
        );
    }

    #[test]
    fn signal_name_maps_known_numbers() {
        assert_eq!(signal_name(1), "SIGHUP");
        assert_eq!(signal_name(2), "SIGINT");
        assert_eq!(signal_name(3), "SIGQUIT");
        assert_eq!(signal_name(9), "SIGKILL");
        assert_eq!(signal_name(15), "SIGTERM");
        assert_eq!(signal_name(42), "SIG42");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminate_group_kills_a_real_process_group() {
        let mut child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 30")
            .process_group(0)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("failed to spawn sleep");
        let pgid = child.id().expect("child has a pid") as i32;

        terminate_group(pgid, Duration::from_millis(300)).await;

        let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .expect("child should have been killed")
            .expect("wait should succeed");
        assert!(!status.success());
    }
}
