use openwebide_core::{
    CommandOutcome, GitCheckoutRequest, GitCheckoutResult, GitCommitRequest, GitCommitResult,
    GitRepoStatus, WebSearchResult,
};
use std::future::Future;

/// Web search and documentation fetching capability for the agent.
pub trait WebClient: Send + Sync {
    fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<WebSearchResult>, String>> + Send;

    fn fetch_page(&self, url: &str) -> impl Future<Output = Result<String, String>> + Send;
}

/// A no-op web client for environments without external web access.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopWebClient;

impl WebClient for NoopWebClient {
    async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<WebSearchResult>, String> {
        Err("Web access is not configured in this environment.".into())
    }

    async fn fetch_page(&self, _url: &str) -> Result<String, String> {
        Err("Web access is not configured in this environment.".into())
    }
}

pub type ContextStatus = (
    Result<openwebide_core::ExecutionEnvironment, String>,
    Result<GitRepoStatus, String>,
);

pub fn context_status_unavailable() -> ContextStatus {
    (
        Err("Execution host metadata timed out".into()),
        Err("Git status timed out".into()),
    )
}

/// Process execution bridge client capability for the agent.
pub trait BridgeClient: Send + Sync {
    /// Best-effort metadata for startup; hosts bound this independently of tool timeouts.
    fn context_status(&self) -> impl Future<Output = ContextStatus> + Send {
        async { futures::future::join(self.environment(), self.git_status()).await }
    }

    fn environment(
        &self,
    ) -> impl Future<Output = Result<openwebide_core::ExecutionEnvironment, String>> + Send {
        async { Err("Execution host unavailable".into()) }
    }

    fn execute_command(
        &self,
        command: &str,
        timeout_seconds: u64,
    ) -> impl Future<Output = Result<CommandOutcome, String>> + Send;

    fn git_status(&self) -> impl Future<Output = Result<GitRepoStatus, String>> + Send {
        async { Err("Git status is not available (bridge daemon not connected).".into()) }
    }

    fn git_diff(&self, path: Option<&str>) -> impl Future<Output = Result<String, String>> + Send {
        let _ = path;
        async { Err("Git diff is not available (bridge daemon not connected).".into()) }
    }

    fn git_commit(
        &self,
        req: &GitCommitRequest,
    ) -> impl Future<Output = Result<GitCommitResult, String>> + Send {
        let _ = req;
        async { Err("Git commit is not available (bridge daemon not connected).".into()) }
    }

    fn git_checkout(
        &self,
        req: &GitCheckoutRequest,
    ) -> impl Future<Output = Result<GitCheckoutResult, String>> + Send {
        let _ = req;
        async { Err("Git checkout is not available (bridge daemon not connected).".into()) }
    }
}

/// A no-op bridge client for environments without an active terminal bridge daemon.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopBridgeClient;

impl BridgeClient for NoopBridgeClient {
    async fn execute_command(
        &self,
        _command: &str,
        _timeout_seconds: u64,
    ) -> Result<CommandOutcome, String> {
        Err("Process execution is not available (bridge daemon not connected). Start 'openwebide-bridge' to enable shell commands.".into())
    }
}

/// Optional execution-host capability. Missing hosts use the same failure contract everywhere.
impl<B: BridgeClient> BridgeClient for Option<B> {
    async fn context_status(&self) -> ContextStatus {
        match self {
            Some(host) => host.context_status().await,
            None => NoopBridgeClient.context_status().await,
        }
    }
    async fn environment(&self) -> Result<openwebide_core::ExecutionEnvironment, String> {
        match self {
            Some(host) => host.environment().await,
            None => NoopBridgeClient.environment().await,
        }
    }
    async fn execute_command(
        &self,
        command: &str,
        timeout_seconds: u64,
    ) -> Result<CommandOutcome, String> {
        match self {
            Some(host) => host.execute_command(command, timeout_seconds).await,
            None => {
                NoopBridgeClient
                    .execute_command(command, timeout_seconds)
                    .await
            }
        }
    }
    async fn git_status(&self) -> Result<GitRepoStatus, String> {
        match self {
            Some(host) => host.git_status().await,
            None => NoopBridgeClient.git_status().await,
        }
    }
    async fn git_diff(&self, path: Option<&str>) -> Result<String, String> {
        match self {
            Some(host) => host.git_diff(path).await,
            None => NoopBridgeClient.git_diff(path).await,
        }
    }
    async fn git_commit(&self, request: &GitCommitRequest) -> Result<GitCommitResult, String> {
        match self {
            Some(host) => host.git_commit(request).await,
            None => NoopBridgeClient.git_commit(request).await,
        }
    }
    async fn git_checkout(
        &self,
        request: &GitCheckoutRequest,
    ) -> Result<GitCheckoutResult, String> {
        match self {
            Some(host) => host.git_checkout(request).await,
            None => NoopBridgeClient.git_checkout(request).await,
        }
    }
}
