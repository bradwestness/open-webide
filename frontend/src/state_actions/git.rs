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

use crate::{project_git::ProjectGit, state::auth::AuthState, workspace::Workspace};

pub struct GitActionContext {
    pub project_git: ProjectGit,
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
    pub fn refresh(
        project_git: ProjectGit,
        projects: ProjectsState,
        git: GitState,
        auth: AuthState,
    ) -> Callback<()> {
        let active_project = projects.active_project;
        let pending = StoredValue::new_local(std::collections::HashSet::new());
        Callback::new(move |()| {
            let Some(project_id) = active_project.get_untracked() else {
                return;
            };
            let generation = auth.generation.get_untracked();
            let revision = project_git.revision();
            let key = (project_id, generation, revision);
            if pending.with_value(|pending| pending.contains(&key)) {
                return;
            }
            pending.update_value(|pending| {
                pending.insert(key);
            });
            spawn_local(async move {
                let result = async {
                    let repository = project_git.repository(Some(project_id)).await?;
                    match futures::future::select(
                        Box::pin(repository.status()),
                        Box::pin(crate::util::sleep_ms(5000)),
                    )
                    .await
                    {
                        futures::future::Either::Left((result, _)) => result,
                        futures::future::Either::Right(_) => Err("Git status timed out".into()),
                    }
                }
                .await;
                if pending
                    .try_update_value(|pending| {
                        pending.remove(&key);
                    })
                    .is_none()
                {
                    return;
                }
                if active_project.try_get_untracked() == Some(Some(project_id))
                    && auth.generation.try_get_untracked() == Some(generation)
                    && project_git.revision() == revision
                {
                    match result {
                        Ok(status) => git.status.set(Some(status)),
                        Err(_) => git.status.set(None),
                    }
                }
            });
        })
    }

    pub fn new(context: GitActionContext) -> Self {
        let GitActionContext {
            project_git,
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
                        match async {
                            project_git
                                .repository(project_id)
                                .await?
                                .checkout(&request)
                                .await
                        }
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
                match async {
                    project_git
                        .repository(project_id)
                        .await?
                        .sync(&request)
                        .await
                }
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

        let on_load_diff = Callback::new(move |()| {
            let Some(file_path) = open_file.get() else {
                return;
            };
            let project_id = active_project.get();
            spawn_local(async move {
                match async {
                    project_git
                        .repository(project_id)
                        .await?
                        .file_head(&file_path)
                        .await
                }
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

        let on_discard_diff = Callback::new(move |()| {
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
                    "Discard all changes to `{head_path}` and restore the committed version?"
                ),
                confirm_label: "Revert".to_string(),
                action: Callback::new(move |()| {
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
