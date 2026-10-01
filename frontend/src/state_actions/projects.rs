use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::WorkspaceMode;

use crate::{api::BackendApi, idb, local_fs};
use openwebide_frontend::state::{
    chat::ChatState,
    git::GitState,
    projects::ProjectsState,
    ui::{ConfirmRequest, UiState},
    workspace::WorkspaceState,
};

pub struct ProjectsActionContext {
    pub api: BackendApi,
    pub projects: ProjectsState,
    pub workspace: WorkspaceState,
    pub git: GitState,
    pub chat: ChatState,
    pub ui: UiState,
    pub ensure_root: Callback<i64>,
    pub refresh_git: Callback<()>,
}

#[derive(Clone, Copy)]
pub struct ProjectsActions {
    pub select_project: Callback<i64>,
    pub close_project: Callback<i64>,
    pub on_open_project: Callback<i64>,
    pub on_open_local: Callback<()>,
    pub on_browser_select: Callback<String>,
    pub on_delete_project: Callback<i64>,
}

pub fn build_projects_actions(context: ProjectsActionContext) -> ProjectsActions {
    let ProjectsActionContext {
        api,
        projects,
        workspace,
        git,
        chat,
        ui,
        ensure_root,
        refresh_git,
    } = context;
    let active_project = projects.active_project;

    let select_project = Callback::new(move |id: i64| {
        let current = active_project.get();
        if current == Some(id) {
            return;
        }
        workspace.switch_project(current, id);
        git.switch_project(current, id);
        chat.active_session
            .set(workspace.active_session.get_untracked());
        ui.clear_toast();
        ensure_root.run(id);
        refresh_git.run(());
    });

    let close_project = Callback::new(move |id: i64| {
        let was_active = active_project.get() == Some(id);
        let next = projects.close_tab(id);
        let next = if was_active {
            // Keep the tab strip's existing policy of selecting the first remaining tab.
            projects.open_tab_ids.get_untracked().first().copied()
        } else {
            next
        };
        if was_active {
            match next {
                Some(next_id) => {
                    workspace.switch_project(Some(id), next_id);
                    git.switch_project(Some(id), next_id);
                    chat.active_session
                        .set(workspace.active_session.get_untracked());
                    ui.clear_toast();
                    ensure_root.run(next_id);
                    refresh_git.run(());
                }
                None => {
                    if let Some(url) = workspace.media_url.get_untracked() {
                        let _ = web_sys::Url::revoke_object_url(&url);
                    }
                    workspace.clear_active();
                    git.clear_active();
                    chat.active_session.set(None);
                }
            }
        }
    });

    let on_open_project = Callback::new(move |id: i64| {
        projects.open_tab(id);
        select_project.run(id);
    });

    let on_open_local = Callback::new(move |_| {
        ui.clear_toast();
        spawn_local(async move {
            let picked = match local_fs::pick_directory().await {
                Ok(picked) => picked,
                Err(error) => {
                    ui.notify(error);
                    return;
                }
            };
            let Some(handle) = picked else {
                return;
            };

            let name = handle.name();
            let project = match api
                .create_project(&name, WorkspaceMode::Local, Some(name.clone()))
                .await
            {
                Ok(project) => project,
                Err(error) => {
                    ui.notify(error);
                    return;
                }
            };

            // Re-picking an existing folder returns its existing project.
            let preexisting = projects
                .projects
                .get()
                .iter()
                .any(|existing| existing.id == project.id);
            if let Err(error) = idb::save_handle(project.id, &handle).await {
                if !preexisting {
                    let _ = api.delete_project(project.id).await;
                }
                ui.notify(error);
                return;
            }

            projects.local_handles.update(|handles| {
                handles.insert(project.id, handle);
            });
            projects.add_project(project.clone());
            projects.open_tab(project.id);
            select_project.run(project.id);
        });
    });

    let on_browser_select = Callback::new(move |path: String| {
        ui.clear_toast();
        spawn_local(async move {
            let name = folder_name(&path);
            let project = match api
                .create_project(&name, WorkspaceMode::Remote, Some(path))
                .await
            {
                Ok(project) => project,
                Err(error) => {
                    ui.notify(error);
                    return;
                }
            };

            projects.add_project(project.clone());
            projects.open_tab(project.id);
            select_project.run(project.id);
        });
    });

    let on_delete_project = Callback::new(move |id: i64| {
        ui.set_confirm(ConfirmRequest {
            title: "Delete project".to_string(),
            message: "Delete this project? Its sessions and messages will be removed too."
                .to_string(),
            confirm_label: "Delete".to_string(),
            action: Callback::new(move |_| {
                spawn_local(async move {
                    if let Err(error) = api.delete_project(id).await {
                        ui.notify(error);
                        return;
                    }
                    workspace.snapshots.update(|snapshots| {
                        if let Some(snapshot) = snapshots.remove(&id)
                            && let Some(url) = snapshot.media_url
                        {
                            let _ = web_sys::Url::revoke_object_url(&url);
                        }
                    });
                    projects.local_handles.update(|handles| {
                        handles.remove(&id);
                    });
                    projects
                        .projects
                        .update(|all| all.retain(|project| project.id != id));
                    chat.sessions.update(|sessions| {
                        sessions.retain(|session| session.project_id != Some(id))
                    });
                    close_project.run(id);
                    git.forget_project(id);
                });
            }),
        });
    });

    ProjectsActions {
        select_project,
        close_project,
        on_open_project,
        on_open_local,
        on_browser_select,
        on_delete_project,
    }
}

fn folder_name(path: &str) -> String {
    path.rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or("source")
        .to_string()
}
