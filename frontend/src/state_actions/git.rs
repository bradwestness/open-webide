use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{GitCheckoutRequest, GitSyncRequest};

use crate::state::{
    chat::ChatState,
    git::{GitState, HeadContent},
    projects::ProjectsState,
    ui::{ConfirmRequest, PromptRequest, UiState},
    workspace::WorkspaceState,
};

use crate::{backend::Api, workspace::Workspace};

pub struct GitActionContext {
    pub api: Api,
    pub projects: ProjectsState,
    pub workspace: WorkspaceState,
    pub git: GitState,
    pub chat: ChatState,
    pub ui: UiState,
    pub workspace_for: Callback<i64, Option<Workspace>>,
    pub refresh: Callback<()>,
}

#[derive(Clone, Copy)]
pub struct GitActions {
    pub on_branch_click: Callback<()>,
    pub on_sync_click: Callback<()>,
    pub on_load_diff: Callback<()>,
    pub on_discard_diff: Callback<()>,
}

impl GitActions {
    pub fn refresh(api: Api, projects: ProjectsState, git: GitState) -> Callback<()> {
        let active_project = projects.active_project;
        Callback::new(move |()| {
            let project_id = active_project.get();
            spawn_local(async move {
                if let Ok(status) = api.with_value(Clone::clone).git_status(project_id).await
                    && active_project.get_untracked() == project_id
                {
                    git.status.set(Some(status));
                }
            });
        })
    }

    pub fn new(context: GitActionContext) -> Self {
        let GitActionContext {
            api,
            projects,
            workspace,
            git,
            chat,
            ui,
            workspace_for,
            refresh,
        } = context;
        let active_project = projects.active_project;
        let open_file = workspace.open_file;
        Effect::new(move |_| {
            let _ = open_file.get();
            git.reset_head_content();
        });

        let on_branch_click = Callback::new(move |()| {
            let project_id = active_project.get();
            ui.set_prompt(PromptRequest {
                title: "Switch or Create Git Branch".to_string(),
                value: String::new(),
                placeholder: "Branch name (e.g. feat/my-feature)".to_string(),
                submit_label: "Switch".to_string(),
                on_submit: Callback::new(move |branch: String| {
                    let branch = branch.trim().to_string();
                    if branch.is_empty() {
                        return;
                    }
                    spawn_local(async move {
                        let request = GitCheckoutRequest {
                            branch,
                            create_if_missing: true,
                        };
                        match api
                            .with_value(Clone::clone)
                            .git_checkout(project_id, &request)
                            .await
                        {
                            Ok(result) => {
                                refresh.run(());
                                chat.notify(GitState::checkout_notice(&result));
                            }
                            Err(error) => ui.notify(format!("Git checkout failed: {error}")),
                        }
                    });
                }),
            });
        });

        let on_sync_click = Callback::new(move |()| {
            let project_id = active_project.get();
            spawn_local(async move {
                let request = GitSyncRequest {
                    action: "sync".into(),
                    remote: None,
                    branch: None,
                };
                match api
                    .with_value(Clone::clone)
                    .git_sync(project_id, &request)
                    .await
                {
                    Ok(result) => {
                        refresh.run(());
                        chat.notify(format!(
                            "Git synchronized with `{}/{}`:\n* Pulled: {} commits\n* Pushed: {} commits",
                            result.remote,
                            result.branch,
                            result.pulled_commits,
                            result.pushed_commits
                        ));
                    }
                    Err(error) => ui.notify(format!("Git sync failed: {error}")),
                }
            });
        });

        let on_load_diff = Callback::new(move |_| {
            let Some(file_path) = open_file.get() else {
                return;
            };
            let project_id = active_project.get();
            spawn_local(async move {
                match api
                    .with_value(Clone::clone)
                    .git_file_head(project_id, &file_path)
                    .await
                {
                    Ok(content) => {
                        if active_project.get_untracked() == project_id
                            && open_file.get_untracked().as_deref() == Some(file_path.as_str())
                        {
                            git.head_content.set(Some(HeadContent {
                                project_id,
                                path: file_path.clone(),
                                content: Ok(content),
                            }));
                        }
                    }
                    Err(error) => {
                        if active_project.get_untracked() == project_id
                            && open_file.get_untracked().as_deref() == Some(file_path.as_str())
                        {
                            git.head_content.set(Some(HeadContent {
                                project_id,
                                path: file_path.clone(),
                                content: Err(error.clone()),
                            }));
                            ui.notify(format!("Could not load HEAD: {error}"));
                        }
                    }
                }
            });
        });

        let on_discard_diff = Callback::new(move |_| {
            let Some(head) = git.head_content.get() else {
                return;
            };
            let Some(project_id) = head.project_id else {
                return;
            };
            let Ok(head_content) = head.content else {
                return;
            };
            let head_path = head.path;
            ui.set_confirm(ConfirmRequest {
                title: "Revert to HEAD".to_string(),
                message: format!(
                    "Discard all changes to `{}` and restore the committed version?",
                    head_path
                ),
                confirm_label: "Revert".to_string(),
                action: Callback::new(move |_| {
                    let content = head_content.clone();
                    let path = head_path.clone();
                    spawn_local(async move {
                        let Some(ws) = workspace_for.run(project_id) else {
                            return;
                        };
                        match ws.write(&path, &content).await {
                            Ok(()) => {
                                if active_project.get_untracked() == Some(project_id)
                                    && open_file.get_untracked().as_deref() == Some(path.as_str())
                                {
                                    workspace.content.set(content);
                                    workspace.dirty.set(false);
                                    refresh.run(());
                                }
                            }
                            Err(error) => ui.notify(error),
                        }
                    });
                }),
            });
        });

        Self {
            on_branch_click,
            on_sync_click,
            on_load_diff,
            on_discard_diff,
        }
    }
}
