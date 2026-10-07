use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::WorkspaceMode;

use crate::state::{
    chat::ChatState,
    git::GitState,
    projects::ProjectsState,
    ui::{ConfirmRequest, UiState},
    workspace::WorkspaceState,
};
use crate::{backend::Api, idb, local_fs};

pub struct ProjectsActionContext {
    pub api: Api,
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
    pub select_chat: Callback<()>,
    pub close_project: Callback<i64>,
    pub tab_action: Callback<(i64, crate::tabs::TabAction)>,
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
    let refresh_pending = super::workspace::pending_refresh(api, projects, workspace, ui);

    let select_project = Callback::new(move |id: i64| {
        let current = active_project.get();
        if current == Some(id) {
            refresh_pending.run(id);
            return;
        }
        workspace.switch_project(current, id);
        git.switch_project(current, id);
        chat.restore_project_session(id);
        ui.clear_toast();
        ensure_root.run(id);
        refresh_pending.run(id);
        refresh_git.run(());
    });

    let select_chat = Callback::new(move |()| {
        let Some(id) = active_project.get_untracked() else {
            return;
        };
        workspace.save_active(id);
        git.save_active(id);
        workspace.clear_active();
        git.clear_active();
        chat.active_editor_context.set(None);
        chat.restore_chat_session();
        ui.clear_toast();
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
                    chat.restore_project_session(next_id);
                    ui.clear_toast();
                    ensure_root.run(next_id);
                    refresh_pending.run(next_id);
                    refresh_git.run(());
                }
                None => {
                    if let Some(url) = workspace.media_url.get_untracked() {
                        let _ = web_sys::Url::revoke_object_url(&url);
                    }
                    workspace.clear_active();
                    git.clear_active();
                    chat.active_editor_context.set(None);
                    chat.restore_chat_session();
                }
            }
        }
    });

    let tab_action = Callback::new(move |(id, action): (i64, crate::tabs::TabAction)| {
        let tabs = projects.open_tab_ids.get_untracked();
        if matches!(
            action,
            crate::tabs::TabAction::MoveLeft | crate::tabs::TabAction::MoveRight
        ) {
            projects
                .open_tab_ids
                .update(|tabs| action.reorder(tabs, &id));
        } else {
            for target in action.targets(&tabs, &id) {
                close_project.run(target);
            }
        }
    });

    let on_open_project = Callback::new(move |id: i64| {
        projects.open_tab(id);
        select_project.run(id);
    });

    let on_open_local = Callback::new(move |()| {
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
                .with_value(Clone::clone)
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
            if let Err(error) = idb::save_handle(project.id, project.user_id, &handle).await {
                if !preexisting {
                    let _ = api
                        .with_value(Clone::clone)
                        .delete_project(project.id)
                        .await;
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
                .with_value(Clone::clone)
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
            action: Callback::new(move |()| {
                spawn_local(async move {
                    if let Err(error) = api.with_value(Clone::clone).delete_project(id).await {
                        ui.notify(error);
                        return;
                    }
                    workspace.begin_pending_refresh(id);
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
                        sessions.retain(|session| session.project_id != Some(id));
                    });
                    close_project.run(id);
                    workspace.snapshots.update(|snapshots| {
                        snapshots.remove(&id);
                    });
                    git.forget_project(id);
                    if let Err(error) = idb::delete_handle(id).await {
                        ui.notify(format!(
                            "Project deleted, but local folder cleanup failed: {error}"
                        ));
                    }
                });
            }),
        });
    });

    ProjectsActions {
        select_project,
        select_chat,
        close_project,
        tab_action,
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
