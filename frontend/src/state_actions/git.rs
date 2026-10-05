use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{GitCheckoutRequest, GitSyncRequest};

use crate::state::{
    chat::ChatState,
    git::{GitState, HeadContent},
    projects::ProjectsState,
    ui::{PromptRequest, UiState},
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
    pub on_load_branches: Callback<()>,
    pub on_select_branch: Callback<String>,
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
            workspace_for: _,
            refresh,
        } = context;
        let active_project = projects.active_project;
        let open_file = workspace.open_file;
        Effect::new(move |_| {
            let _ = open_file.get();
            git.reset_head_content();
        });

        let auth = expect_context::<AuthState>();
        let settings = expect_context::<crate::state::settings::SettingsState>();
        let on_load_branches = Callback::new(move |()| {
            let Some(project_id) = active_project.get_untracked() else {
                return;
            };
            if git.branches_loading.get_untracked() {
                return;
            }
            let generation = auth.generation.get_untracked();
            let host_revision = project_git.revision();
            let revision = git.branch_revision.get_untracked();
            git.branches_loading.set(true);
            git.branches_error.set(None);
            spawn_local(async move {
                let result = match futures::future::select(
                    Box::pin(async {
                        project_git
                            .repository(Some(project_id))
                            .await?
                            .branches()
                            .await
                    }),
                    Box::pin(crate::util::sleep_ms(5000)),
                )
                .await
                {
                    futures::future::Either::Left((result, _)) => result,
                    futures::future::Either::Right(_) => {
                        Err("Git branch discovery timed out".into())
                    }
                };
                if active_project.try_get_untracked() != Some(Some(project_id))
                    || auth.generation.try_get_untracked() != Some(generation)
                    || project_git.revision() != host_revision
                    || git.branch_revision.try_get_untracked() != Some(revision)
                {
                    return;
                }
                git.branches_loading.set(false);
                match result {
                    Ok(mut branches) => {
                        branches.retain(|branch| !branch.is_remote);
                        branches.sort_by(|a, b| a.name.cmp(&b.name));
                        git.branches.set(branches);
                    }
                    Err(error) => git.branches_error.set(Some(error)),
                }
            });
        });
        Effect::new(move |_| {
            active_project.track();
            auth.generation.track();
            settings.bridge_url.track();
            projects.local_handles.track();
            git.reset_branches();
        });
        let current_branch = Memo::new(move |_| {
            git.status
                .with(|status| status.as_ref().map(|status| status.branch.clone()))
        });
        Effect::new(move |_| {
            active_project.track();
            auth.generation.track();
            settings.bridge_url.track();
            projects.local_handles.track();
            let branch = current_branch.get();
            if branch.is_some() {
                on_load_branches.run(());
            }
        });
        let checkout = Callback::new(move |(branch, create_if_missing): (String, bool)| {
            let Some(project_id) = active_project.get_untracked() else {
                return;
            };
            if git.branch_busy.get_untracked() {
                return;
            }
            let branch = branch.trim().to_string();
            if branch.is_empty() {
                return;
            }
            let generation = auth.generation.get_untracked();
            let host_revision = project_git.revision();
            let revision = git.branch_revision.get_untracked();
            git.branch_busy.set(true);
            spawn_local(async move {
                let result = async {
                    project_git
                        .repository(Some(project_id))
                        .await?
                        .checkout(&GitCheckoutRequest {
                            branch,
                            create_if_missing,
                        })
                        .await
                }
                .await;
                if active_project.try_get_untracked() != Some(Some(project_id))
                    || auth.generation.try_get_untracked() != Some(generation)
                    || project_git.revision() != host_revision
                    || git.branch_revision.try_get_untracked() != Some(revision)
                {
                    return;
                }
                git.branch_busy.set(false);
                match result {
                    Ok(result) => {
                        git.status.update(|status| {
                            if let Some(status) = status {
                                status.branch.clone_from(&result.branch);
                            }
                        });
                        git.reset_head_content();
                        refresh.run(());
                        on_load_branches.run(());
                        chat.notify(GitState::checkout_notice(&result));
                    }
                    Err(error) => ui.notify(format!("Git checkout failed: {error}")),
                }
            });
        });
        let on_select_branch = Callback::new(move |branch: String| checkout.run((branch, false)));
        let prompt_owner = Owner::current().expect("Git actions have an owner");
        let on_branch_click = Callback::new(move |()| {
            let project_id = active_project.get_untracked();
            let generation = auth.generation.get_untracked();
            let host_revision = project_git.revision();
            let revision = git.branch_revision.get_untracked();
            ui.set_prompt(PromptRequest {
                title: "New Git Branch".to_string(),
                value: String::new(),
                placeholder: "Branch name (e.g. feat/my-feature)".to_string(),
                submit_label: "Create".to_string(),
                on_submit: prompt_owner.with(|| {
                    Callback::new(move |branch: String| {
                        if active_project.get_untracked() == project_id
                            && auth.generation.get_untracked() == generation
                            && project_git.revision() == host_revision
                            && git.branch_revision.get_untracked() == revision
                        {
                            checkout.run((branch, true));
                        }
                    })
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

        let tree_actions = use_context::<super::file_tree::FileTreeActions>();
        let on_discard_diff = Callback::new(move |()| {
            if let Some(actions) = tree_actions
                && let Some(head) = git.head_content.get_untracked()
                && head.project_id == active_project.get_untracked()
            {
                actions.git_action(&head.path, openwebide_core::git::GitPathAction::Revert);
            }
        });

        Self {
            on_branch_click,
            on_load_branches,
            on_select_branch,
            on_sync_click,
            on_load_diff,
            on_discard_diff,
        }
    }
}
