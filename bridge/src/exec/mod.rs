use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use crate::BridgeError;
use openwebide_core::{CommandOutcome, GitCheckoutRequest, GitCommitRequest, GitSyncRequest};

pub mod git;
mod host;
mod host_info;
pub mod proc;

pub type ExecOutput = CommandOutcome;
pub type ExecutionFuture<T> = Pin<Box<dyn Future<Output = Result<T, BridgeError>> + Send>>;

pub fn environment() -> openwebide_core::ExecutionEnvironment {
    openwebide_core::ExecutionEnvironment {
        os: std::env::consts::OS.into(),
        shell: if cfg!(windows) { "cmd" } else { "sh" }.into(),
    }
}

pub struct SpawnSpec {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    pub timeout: Duration,
    pub cancel: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl SpawnSpec {
    pub fn shell(
        command: String,
        cwd: PathBuf,
        timeout_seconds: u64,
        cancel: impl Future<Output = ()> + Send + 'static,
    ) -> Self {
        #[cfg(unix)]
        let (shell, flag) = ("sh", "-c");
        #[cfg(windows)]
        let (shell, flag) = ("cmd", "/C");
        Self {
            command: shell.into(),
            args: vec![flag.into(), command],
            cwd,
            env: HashMap::new(),
            timeout: Duration::from_secs(timeout_seconds),
            cancel: Box::pin(cancel),
        }
    }

    pub fn in_root(
        command: String,
        args: Vec<String>,
        cwd: Option<&str>,
        env: HashMap<String, String>,
        root: &Path,
    ) -> Result<Self, BridgeError> {
        Ok(Self {
            command,
            args,
            cwd: crate::paths::resolve_in_root(root, cwd).map_err(BridgeError::Validation)?,
            env,
            timeout: Duration::from_secs(30),
            cancel: Box::pin(std::future::pending()),
        })
    }

    pub fn display_cmd(&self) -> String {
        if self.args.is_empty() {
            self.command.clone()
        } else {
            format!("{} {}", self.command, self.args.join(" "))
        }
    }
}

pub struct GitRequest {
    pub cwd: PathBuf,
    pub operation: GitOperation,
}

pub enum GitOperation {
    Status,
    Diff(Option<String>),
    Show(String),
    Branches,
    Commit(GitCommitRequest),
    Checkout(GitCheckoutRequest),
    Sync(GitSyncRequest),
}

/// Typed execution results; serialization preserves each existing HTTP route's JSON shape.
#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum GitResponse {
    Status(openwebide_core::GitRepoStatus),
    Diff(openwebide_core::GitDiff),
    Show(openwebide_core::GitFileContent),
    Branches(Vec<openwebide_core::GitBranchInfo>),
    Commit(openwebide_core::GitCommitResult),
    Checkout(openwebide_core::GitCheckoutResult),
    Sync(openwebide_core::GitSyncResult),
}

pub trait ToolExecution: Send + Sync {
    fn host_info(&self) -> ExecutionFuture<openwebide_core::HostInfo> {
        Box::pin(async {
            Err(BridgeError::Execution(
                "Host information is unavailable".into(),
            ))
        })
    }
    fn run_command(&self, spec: SpawnSpec) -> ExecutionFuture<ExecOutput>;
    fn git(&self, request: GitRequest) -> ExecutionFuture<GitResponse>;
}

#[derive(Default)]
pub struct HostExecution;

impl ToolExecution for HostExecution {
    fn host_info(&self) -> ExecutionFuture<openwebide_core::HostInfo> {
        Box::pin(async {
            tokio::time::timeout(Duration::from_secs(6), host_info::snapshot())
                .await
                .map_err(|_| BridgeError::Execution("Host information timed out".into()))?
        })
    }
    fn run_command(&self, spec: SpawnSpec) -> ExecutionFuture<ExecOutput> {
        Box::pin(host::execute_command_direct(spec))
    }

    fn git(&self, request: GitRequest) -> ExecutionFuture<GitResponse> {
        Box::pin(async move {
            let dir = &request.cwd;
            Ok(match request.operation {
                GitOperation::Status => GitResponse::Status(git::get_repo_status(dir).await?),
                GitOperation::Diff(path) => GitResponse::Diff(openwebide_core::GitDiff {
                    diff: git::get_repo_diff(dir, path.as_deref()).await?,
                }),
                GitOperation::Show(path) => {
                    GitResponse::Show(git::get_file_at_head(dir, &path).await?)
                }
                GitOperation::Branches => GitResponse::Branches(git::get_repo_branches(dir).await?),
                GitOperation::Commit(req) => {
                    GitResponse::Commit(git::commit_changes(dir, &req).await?)
                }
                GitOperation::Checkout(req) => {
                    GitResponse::Checkout(git::checkout_branch(dir, &req).await?)
                }
                GitOperation::Sync(req) => GitResponse::Sync(git::sync_repo(dir, &req).await?),
            })
        })
    }
}
