//! Shared Git workflows select their transport through the project execution facade.
use crate::{
    backend::Api,
    local_agent::BrowserBridgeClient,
    project_host::{ProjectExecution, ProjectHost},
    state::{auth::AuthState, projects::ProjectsState, settings::SettingsState},
};
use leptos::prelude::*;
use openwebide_agent::BridgeClient;
use openwebide_core::{
    GitBranchInfo, GitCheckoutRequest, GitCheckoutResult, GitCommitRequest, GitCommitResult,
    GitRepoStatus, GitSyncRequest, GitSyncResult,
};

#[derive(Clone, Copy)]
pub struct ProjectGit {
    host: ProjectHost,
}
impl ProjectGit {
    pub fn new(
        api: Api,
        projects: ProjectsState,
        settings: SettingsState,
        auth: AuthState,
    ) -> Self {
        let host = ProjectHost::new(api, projects, settings, auth);
        provide_context(host);
        Self { host }
    }
    pub fn revision(self) -> Option<u64> {
        self.host.revision()
    }
    pub async fn repository(self, project: Option<i64>) -> Result<GitRepository, String> {
        match self.host.resolve(project).await? {
            ProjectExecution::Remote {
                api, project_id, ..
            } => Ok(GitRepository::Remote { api, project_id }),
            ProjectExecution::Local(client) => Ok(GitRepository::Local(client)),
        }
    }
}

pub enum GitRepository {
    Remote { api: Api, project_id: Option<i64> },
    Local(BrowserBridgeClient),
}

impl GitRepository {
    pub async fn status(&self) -> Result<GitRepoStatus, String> {
        match self {
            Self::Remote { api, project_id } => {
                api.with_value(Clone::clone).git_status(*project_id).await
            }
            Self::Local(client) => client.git_status().await,
        }
    }

    pub async fn diff(&self, path: Option<&str>) -> Result<String, String> {
        match self {
            Self::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .git_diff(*project_id, path)
                    .await
            }
            Self::Local(client) => client.git_diff(path).await,
        }
    }

    pub async fn file_head(&self, path: &str) -> Result<String, String> {
        match self {
            Self::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .git_file_head(*project_id, path)
                    .await
            }
            Self::Local(client) => {
                let value: openwebide_core::GitFileContent = client
                    .git_request("show", serde_json::json!({ "path": path }))
                    .await?;
                value.into_text()
            }
        }
    }

    pub async fn branches(&self) -> Result<Vec<GitBranchInfo>, String> {
        match self {
            Self::Remote { api, project_id } => {
                api.with_value(Clone::clone).git_branches(*project_id).await
            }
            Self::Local(client) => client.git_request("branches", serde_json::json!({})).await,
        }
    }

    pub async fn commit(&self, request: &GitCommitRequest) -> Result<GitCommitResult, String> {
        match self {
            Self::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .git_commit(*project_id, request)
                    .await
            }
            Self::Local(client) => client.git_commit(request).await,
        }
    }

    pub async fn checkout(
        &self,
        request: &GitCheckoutRequest,
    ) -> Result<GitCheckoutResult, String> {
        match self {
            Self::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .git_checkout(*project_id, request)
                    .await
            }
            Self::Local(client) => client.git_checkout(request).await,
        }
    }

    pub async fn sync(&self, request: &GitSyncRequest) -> Result<GitSyncResult, String> {
        match self {
            Self::Remote { api, project_id } => {
                api.with_value(Clone::clone)
                    .git_sync(*project_id, request)
                    .await
            }
            Self::Local(client) => {
                client
                    .git_request(
                        "sync",
                        serde_json::to_value(request).map_err(|error| error.to_string())?,
                    )
                    .await
            }
        }
    }
}
