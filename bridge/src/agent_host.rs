use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use openwebide_agent::{BridgeClient, CancelCheck, PermissionGate, WebClient};
use openwebide_core::{
    CommandOutcome, GitCheckoutRequest, GitCheckoutResult, GitCommitRequest, GitCommitResult,
    GitRepoStatus, ToolCall, WebSearchResult,
};
use tokio::sync::{Notify, oneshot};

use crate::backend_client::RunBackend;

pub struct InProcessBridgeClient {
    pub dir: PathBuf,
    pub cancel: BridgeCancel,
}

impl BridgeClient for InProcessBridgeClient {
    async fn execute_command(
        &self,
        command: &str,
        timeout_seconds: u64,
    ) -> Result<CommandOutcome, String> {
        crate::headless::execute_command_direct(
            command,
            &self.dir,
            timeout_seconds,
            self.cancel.cancelled(),
        )
        .await
    }
    async fn git_status(&self) -> Result<GitRepoStatus, String> {
        crate::git::get_repo_status(&self.dir)
            .await
            .map_err(|e| e.to_string())
    }
    async fn git_diff(&self, path: Option<&str>) -> Result<String, String> {
        crate::git::get_repo_diff(&self.dir, path)
            .await
            .map_err(|e| e.to_string())
    }
    async fn git_commit(&self, req: &GitCommitRequest) -> Result<GitCommitResult, String> {
        crate::git::commit_changes(&self.dir, req)
            .await
            .map_err(|e| e.to_string())
    }
    async fn git_checkout(&self, req: &GitCheckoutRequest) -> Result<GitCheckoutResult, String> {
        crate::git::checkout_branch(&self.dir, req)
            .await
            .map_err(|e| e.to_string())
    }
}

pub struct BackendWebClient<B> {
    pub backend: Arc<B>,
    pub user_id: i64,
}

impl<B: RunBackend> WebClient for BackendWebClient<B> {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<WebSearchResult>, String> {
        self.backend.web_search(self.user_id, query, limit).await
    }
    async fn fetch_page(&self, url: &str) -> Result<String, String> {
        self.backend.web_fetch(self.user_id, url).await
    }
}

#[derive(Clone, Debug, Default)]
pub struct BridgeCancel {
    flag: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl BridgeCancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
    pub async fn cancelled(&self) {
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !self.is_cancelled() {
            notified.await;
        }
    }
}

impl CancelCheck for BridgeCancel {
    async fn cancelled(&self) {
        BridgeCancel::cancelled(self).await;
    }
    async fn check(&self) -> bool {
        self.is_cancelled()
    }
}

#[derive(Debug)]
struct PendingPermission {
    id: String,
    sender: Option<oneshot::Sender<bool>>,
    receiver: Option<oneshot::Receiver<bool>>,
}

#[derive(Clone, Debug)]
pub struct BridgeGate {
    pending: Arc<Mutex<Option<PendingPermission>>>,
    cancel: BridgeCancel,
}

impl BridgeGate {
    pub fn new(cancel: BridgeCancel) -> Self {
        Self {
            pending: Arc::new(Mutex::new(None)),
            cancel,
        }
    }
    pub fn decide(&self, id: &str, approved: bool) -> Result<(), String> {
        let mut pending = self.pending.lock().unwrap();
        let current = pending
            .as_mut()
            .filter(|p| p.id == id)
            .ok_or_else(|| format!("no pending permission for {id}"))?;
        let sender = current
            .sender
            .take()
            .ok_or_else(|| format!("no pending permission for {id}"))?;
        sender
            .send(approved)
            .map_err(|_| format!("no pending permission for {id}"))
    }
    pub fn clear(&self) {
        self.pending.lock().unwrap().take();
    }
    // Register before publishing the permission event, so an immediate reply cannot be lost.
    pub fn prepare(&self, id: &str) {
        let (sender, receiver) = oneshot::channel();
        *self.pending.lock().unwrap() = Some(PendingPermission {
            id: id.to_string(),
            sender: Some(sender),
            receiver: Some(receiver),
        });
    }
}

impl PermissionGate for BridgeGate {
    async fn approve(&self, call: &ToolCall) -> bool {
        let receiver = {
            if self.pending.lock().unwrap().is_none() {
                self.prepare(&call.id);
            }
            self.pending
                .lock()
                .unwrap()
                .as_mut()
                .unwrap()
                .receiver
                .take()
                .unwrap()
        };
        let decision = tokio::select! {
            result = receiver => result.unwrap_or(false),
            _ = self.cancel.cancelled() => false,
            _ = tokio::time::sleep(Duration::from_secs(300)) => false,
        };
        self.pending.lock().unwrap().take();
        decision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call() -> ToolCall {
        ToolCall {
            id: "t".into(),
            name: "write_file".into(),
            arguments: "{}".into(),
        }
    }

    #[tokio::test]
    async fn permission_ids_are_pending_and_decisions_used_once() {
        let gate = BridgeGate::new(BridgeCancel::default());
        assert!(gate.decide("t", true).is_err());
        gate.prepare("t");
        assert!(gate.decide("other", true).is_err());
        gate.decide("t", true).unwrap();
        assert!(gate.decide("t", false).is_err());
        assert!(gate.approve(&call()).await);
        assert!(gate.decide("t", true).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn permission_timeout_denies_and_cancel_wakes_waiter() {
        let cancel = BridgeCancel::default();
        let gate = BridgeGate::new(cancel.clone());
        let waiter_gate = gate.clone();
        let waiter = tokio::spawn(async move { waiter_gate.approve(&call()).await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(300)).await;
        assert!(!waiter.await.unwrap());
        assert!(gate.decide("t", true).is_err());
        let waiter = tokio::spawn(async move { gate.approve(&call()).await });
        tokio::task::yield_now().await;
        cancel.cancel();
        assert!(!waiter.await.unwrap());
    }
}
