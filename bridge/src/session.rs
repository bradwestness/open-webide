//! Process session management, ring buffering, and client broadcasting.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use openwebide_core::{BridgeServerMessage, BridgeSessionInfo};
use tokio::sync::{broadcast, mpsc};

use crate::proc::Signal;

/// Maximum size of the output ring buffer in bytes (2MB default per session).
const MAX_RING_BUFFER_BYTES: usize = 2 * 1024 * 1024;

/// A circular buffer preserving output chunks with monotonic sequence numbers
/// to enable reconnects and missed-message replays.
pub struct OutputRingBuffer {
    chunks: VecDeque<BridgeServerMessage>,
    total_bytes: usize,
    next_seq: u64,
}

impl Default for OutputRingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputRingBuffer {
    pub fn new() -> Self {
        Self {
            chunks: VecDeque::new(),
            total_bytes: 0,
            next_seq: 1,
        }
    }

    /// Push an output chunk or terminal event into the ring.
    pub fn push(&mut self, id: &str, stream: &str, data: &str) -> BridgeServerMessage {
        let seq = self.next_seq;
        self.next_seq += 1;

        let msg = BridgeServerMessage::Output {
            id: id.to_string(),
            seq,
            stream: stream.to_string(),
            data: data.to_string(),
        };

        let chunk_bytes = data.len();
        self.chunks.push_back(msg.clone());
        self.total_bytes += chunk_bytes;

        while self.total_bytes > MAX_RING_BUFFER_BYTES && self.chunks.len() > 1 {
            if let Some(removed) = self.chunks.pop_front()
                && let BridgeServerMessage::Output { data, .. } = removed
            {
                self.total_bytes = self.total_bytes.saturating_sub(data.len());
            }
        }

        msg
    }

    /// Replay all messages with sequence numbers greater than `last_seq`.
    pub fn replay_after(&self, last_seq: u64) -> Vec<BridgeServerMessage> {
        self.chunks
            .iter()
            .filter(|msg| match msg {
                BridgeServerMessage::Output { seq, .. } => *seq > last_seq,
                _ => true,
            })
            .cloned()
            .collect()
    }
}

/// An active process or interactive PTY session.
pub struct Session {
    pub id: String,
    pub command: String,
    pub pty: bool,
    pub started_at: u64,
    /// The OS pid of the spawned child, which is also its process group id: every session is
    /// spawned as its own group leader (`process_group(0)`, or a PTY session leader via
    /// `setsid`). `None` on platforms without process groups (Windows).
    pub pid: Option<i32>,
    pub running: Arc<AtomicBool>,
    pub exit_code: Arc<Mutex<Option<i32>>>,
    pub exited_at: Mutex<Option<Instant>>,
    pub ring: Mutex<OutputRingBuffer>,
    pub broadcast_tx: broadcast::Sender<BridgeServerMessage>,
    stdin_tx: Mutex<Option<mpsc::Sender<String>>>,
    resize_tx: Mutex<Option<mpsc::Sender<(u16, u16)>>>,
    kill_tx: Mutex<Option<mpsc::Sender<Signal>>>,
}

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        command: String,
        pty: bool,
        pid: Option<i32>,
        stdin_tx: mpsc::Sender<String>,
        resize_tx: mpsc::Sender<(u16, u16)>,
        kill_tx: mpsc::Sender<Signal>,
    ) -> Self {
        let (broadcast_tx, _) = broadcast::channel(1024);
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();

        Self {
            id,
            command,
            pty,
            started_at,
            pid,
            running: Arc::new(AtomicBool::new(true)),
            exit_code: Arc::new(Mutex::new(None)),
            exited_at: Mutex::new(None),
            ring: Mutex::new(OutputRingBuffer::new()),
            broadcast_tx,
            stdin_tx: Mutex::new(Some(stdin_tx)),
            resize_tx: Mutex::new(Some(resize_tx)),
            kill_tx: Mutex::new(Some(kill_tx)),
        }
    }

    /// Send input data to the process's stdin or PTY. A no-op once the session has exited.
    pub async fn send_input(&self, data: String) {
        let tx = self.stdin_tx.lock().unwrap().clone();
        if let Some(tx) = tx {
            let _ = tx.send(data).await;
        }
    }

    /// Resize the PTY. A no-op for headless sessions or once the session has exited.
    pub async fn resize(&self, cols: u16, rows: u16) {
        let tx = self.resize_tx.lock().unwrap().clone();
        if let Some(tx) = tx {
            let _ = tx.send((cols, rows)).await;
        }
    }

    /// Request the process be sent `signal`. A no-op once the session has exited.
    pub async fn kill(&self, signal: Signal) {
        let tx = self.kill_tx.lock().unwrap().clone();
        if let Some(tx) = tx {
            let _ = tx.send(signal).await;
        }
    }

    /// Append output to buffer and notify all active subscribers.
    pub fn emit_output(&self, stream: &str, data: &str) {
        let msg = self.ring.lock().unwrap().push(&self.id, stream, data);
        let _ = self.broadcast_tx.send(msg);
    }

    /// Mark the session as terminated with the final exit code, and drop the input/resize/kill
    /// senders so their receiving tasks end and the underlying fds (PTY master, child stdin)
    /// close.
    pub fn emit_exit(&self, code: Option<i32>, signal: Option<String>) {
        self.running.store(false, Ordering::SeqCst);
        *self.exit_code.lock().unwrap() = code;
        *self.exited_at.lock().unwrap() = Some(Instant::now());
        self.stdin_tx.lock().unwrap().take();
        self.resize_tx.lock().unwrap().take();
        self.kill_tx.lock().unwrap().take();
        let msg = BridgeServerMessage::Exited {
            id: self.id.clone(),
            exit_code: code,
            signal,
        };
        let _ = self.broadcast_tx.send(msg);
    }

    /// Emit an error message.
    pub fn emit_error(&self, message: &str) {
        let msg = BridgeServerMessage::Error {
            id: self.id.clone(),
            message: message.to_string(),
        };
        let _ = self.broadcast_tx.send(msg);
    }

    /// Attach a client to this session, returning buffered replay chunks and a live receiver.
    pub fn attach(
        &self,
        last_seq: u64,
    ) -> (
        Vec<BridgeServerMessage>,
        broadcast::Receiver<BridgeServerMessage>,
    ) {
        let replay = self.ring.lock().unwrap().replay_after(last_seq);
        let rx = self.broadcast_tx.subscribe();
        (replay, rx)
    }

    pub fn to_info(&self) -> BridgeSessionInfo {
        BridgeSessionInfo {
            id: self.id.clone(),
            command: self.command.clone(),
            running: self.running.load(Ordering::SeqCst),
            pty: self.pty,
            started_at: self.started_at,
        }
    }
}

/// Global session manager overseeing all active sessions.
#[derive(Clone, Default)]
pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<String, Arc<Session>>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn insert(&self, session: Arc<Session>) {
        self.sessions
            .write()
            .unwrap()
            .insert(session.id.clone(), session);
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.sessions.read().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<BridgeSessionInfo> {
        self.sessions
            .read()
            .unwrap()
            .values()
            .map(|s| s.to_info())
            .collect()
    }

    pub fn remove(&self, id: &str) {
        self.sessions.write().unwrap().remove(id);
    }

    /// Remove every exited session whose `exited_at` is at least `ttl` old. Running sessions are
    /// never reaped, however long they've been alive. Returns how many were removed.
    pub fn reap(&self, ttl: Duration) -> usize {
        let mut sessions = self.sessions.write().unwrap();
        let expired: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| {
                !s.running.load(Ordering::SeqCst)
                    && s.exited_at
                        .lock()
                        .unwrap()
                        .is_some_and(|t| t.elapsed() >= ttl)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let count = expired.len();
        for id in expired {
            sessions.remove(&id);
        }
        count
    }

    /// Terminate every running session's process group concurrently, for graceful daemon
    /// shutdown.
    pub async fn kill_all(&self) {
        #[cfg(unix)]
        {
            let sessions: Vec<Arc<Session>> =
                self.sessions.read().unwrap().values().cloned().collect();
            let kills = sessions
                .iter()
                .filter(|s| s.running.load(Ordering::SeqCst))
                .filter_map(|s| s.pid)
                .map(|pgid| crate::proc::terminate_group(pgid, Duration::from_secs(2)));
            futures::future::join_all(kills).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_seq_and_replay() {
        let mut buf = OutputRingBuffer::new();
        assert_eq!(buf.chunks.len(), 0);

        let msg1 = buf.push("sess-1", "stdout", "hello ");
        let msg2 = buf.push("sess-1", "stdout", "world\n");

        if let BridgeServerMessage::Output { seq, .. } = msg1 {
            assert_eq!(seq, 1);
        } else {
            panic!("expected Output message");
        }

        if let BridgeServerMessage::Output { seq, .. } = msg2 {
            assert_eq!(seq, 2);
        } else {
            panic!("expected Output message");
        }

        // Replay all from beginning (last_seq = 0)
        let all = buf.replay_after(0);
        assert_eq!(all.len(), 2);
        match &all[0] {
            BridgeServerMessage::Output {
                seq,
                data,
                stream,
                id,
            } => {
                assert_eq!(*seq, 1);
                assert_eq!(data, "hello ");
                assert_eq!(stream, "stdout");
                assert_eq!(id, "sess-1");
            }
            _ => panic!("unexpected message"),
        }

        // Replay after seq 1
        let after_1 = buf.replay_after(1);
        assert_eq!(after_1.len(), 1);
        match &after_1[0] {
            BridgeServerMessage::Output { seq, data, .. } => {
                assert_eq!(*seq, 2);
                assert_eq!(data, "world\n");
            }
            _ => panic!("unexpected message"),
        }

        // Replay up to date (last_seq = 2)
        let up_to_date = buf.replay_after(2);
        assert!(up_to_date.is_empty());
    }

    fn test_session(id: &str) -> Arc<Session> {
        let (stdin_tx, _) = mpsc::channel(1);
        let (resize_tx, _) = mpsc::channel(1);
        let (kill_tx, _) = mpsc::channel(1);
        Arc::new(Session::new(
            id.into(),
            "echo test".into(),
            false,
            Some(1234),
            stdin_tx,
            resize_tx,
            kill_tx,
        ))
    }

    #[test]
    fn test_session_manager() {
        let mgr = SessionManager::new();
        let sess = test_session("sess-1");

        mgr.insert(sess);
        assert!(mgr.get("sess-1").is_some());
        assert_eq!(mgr.list().len(), 1);
        assert_eq!(mgr.list()[0].id, "sess-1");

        mgr.remove("sess-1");
        assert!(mgr.get("sess-1").is_none());
        assert_eq!(mgr.list().len(), 0);
    }

    #[test]
    fn emit_exit_drops_senders() {
        let sess = test_session("sess-1");
        assert!(sess.stdin_tx.lock().unwrap().is_some());
        assert!(sess.resize_tx.lock().unwrap().is_some());
        assert!(sess.kill_tx.lock().unwrap().is_some());

        sess.emit_exit(Some(0), None);

        assert!(!sess.running.load(Ordering::SeqCst));
        assert!(sess.exited_at.lock().unwrap().is_some());
        assert!(sess.stdin_tx.lock().unwrap().is_none());
        assert!(sess.resize_tx.lock().unwrap().is_none());
        assert!(sess.kill_tx.lock().unwrap().is_none());
    }

    #[test]
    fn reap_removes_only_expired_exited() {
        let mgr = SessionManager::new();

        let still_running = test_session("running");
        mgr.insert(still_running);

        let freshly_exited = test_session("fresh");
        freshly_exited.emit_exit(Some(0), None);
        mgr.insert(freshly_exited);

        let long_exited = test_session("stale");
        long_exited.emit_exit(Some(0), None);
        *long_exited.exited_at.lock().unwrap() = Some(Instant::now() - Duration::from_secs(3600));
        mgr.insert(long_exited);

        let removed = mgr.reap(Duration::from_secs(60));

        assert_eq!(removed, 1);
        assert!(mgr.get("running").is_some());
        assert!(mgr.get("fresh").is_some());
        assert!(mgr.get("stale").is_none());
    }
}
