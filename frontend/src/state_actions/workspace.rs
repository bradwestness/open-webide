use crate::state::{
    projects::ProjectsState,
    ui::{ConfirmRequest, PromptRequest, UiState},
    workspace::WorkspaceState,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{FileKind, vfs::SearchOptions};

use crate::{backend::Api, local_fs, workspace::Workspace};

/// Browser and backend actions for opening, editing, and searching files.
#[derive(Clone, Copy)]
pub struct WorkspaceActions {
    pub workspace_for: Callback<i64, Option<Workspace>>,
    pub on_grant_access: Callback<()>,
    pub ensure_root: Callback<i64>,
    pub on_open_lossy: Callback<()>,
    pub request_open: Callback<String>,
    pub on_toggle: Callback<String>,
    pub on_save: Callback<()>,
    pub on_accept: Callback<()>,
    pub on_reject: Callback<()>,
    pub on_new_file: Callback<()>,
    pub on_new_dir: Callback<()>,
    pub on_search: Callback<(String, SearchOptions)>,
    pub on_clear_search: Callback<()>,
}

impl WorkspaceActions {
    pub fn new(
        api: Api,
        projects: ProjectsState,
        workspace: WorkspaceState,
        ui: UiState,
        read_only: RwSignal<bool>,
        refresh_git: Callback<()>,
    ) -> Self {
        let active_project = projects.active_project;
        let workspace_for = Callback::new(move |project_id: i64| -> Option<Workspace> {
            let project = projects.project(project_id)?;
            match project.mode {
                openwebide_core::WorkspaceMode::Remote => {
                    Some(Workspace::Remote { api, project_id })
                }
                openwebide_core::WorkspaceMode::Local => projects
                    .local_handles
                    .with(|handles| handles.get(&project_id).cloned())
                    .map(|handle| Workspace::Local { handle }),
            }
        });

        let load_dir = Callback::new(move |(project_id, dir): (i64, String)| {
            spawn_local(async move {
                let Some(ws) = workspace_for.run(project_id) else {
                    if active_project.get() == Some(project_id) {
                        ui.toast.set(Some(
                            "Folder not available. Re-open the project to pick it again."
                                .to_string(),
                        ));
                    }
                    return;
                };
                match ws.list(&dir).await {
                    Ok(entries) => {
                        if active_project.get() == Some(project_id) {
                            workspace.entries.update(|entries_by_dir| {
                                entries_by_dir.insert(dir, entries);
                            });
                        }
                    }
                    Err(error) => {
                        if error == local_fs::PERMISSION_NEEDED {
                            projects.needs_grant.update(|projects_needing_grant| {
                                projects_needing_grant.insert(project_id);
                            });
                        } else if active_project.get() == Some(project_id) {
                            ui.toast.set(Some(error));
                        }
                    }
                }
            });
        });

        let on_grant_access = Callback::new(move |()| {
            let Some(project_id) = active_project.get() else {
                return;
            };
            let handle = projects
                .local_handles
                .with(|handles| handles.get(&project_id).cloned());
            if let Some(handle) = handle {
                let load_dir = load_dir;
                spawn_local(async move {
                    if let Ok(true) = local_fs::request_access(&handle).await {
                        projects.needs_grant.update(|projects_needing_grant| {
                            projects_needing_grant.remove(&project_id);
                        });
                        load_dir.run((project_id, String::new()));
                    }
                });
            }
        });

        let ensure_root = Callback::new(move |project_id: i64| {
            if workspace.entries.get().contains_key("") {
                return;
            }
            load_dir.run((project_id, String::new()));
        });

        let on_open_lossy = Callback::new(move |()| {
            let Some(project_id) = active_project.get() else {
                return;
            };
            let Some(path) = workspace.open_file.get() else {
                return;
            };
            read_only.set(true);
            workspace.dirty.set(false);
            spawn_local(async move {
                let Some(ws) = workspace_for.run(project_id) else {
                    return;
                };
                let result = match ws {
                    Workspace::Remote { api, project_id } => {
                        api.with_value(Clone::clone)
                            .read_file_lossy(project_id, &path)
                            .await
                    }
                    Workspace::Local { handle } => local_fs::read_lossy(&handle, &path).await,
                };
                match result {
                    Ok(content) => {
                        if active_project.get_untracked() == Some(project_id)
                            && workspace.open_file.get_untracked().as_deref() == Some(path.as_str())
                        {
                            workspace.content.set(content);
                        }
                    }
                    Err(error) => {
                        if active_project.get_untracked() == Some(project_id)
                            && workspace.open_file.get_untracked().as_deref() == Some(path.as_str())
                        {
                            ui.toast.set(Some(error));
                        }
                    }
                }
            });
        });

        let on_open = Callback::new(move |path: String| {
            let Some(project_id) = active_project.get() else {
                return;
            };
            read_only.set(false);
            workspace.dirty.set(false);
            workspace.open_file.set(Some(path.clone()));
            workspace.content.set(String::new());
            revoke_object_url(workspace.media_url.get());
            workspace.media_url.set(None);
            ui.toast.set(None);
            let kind = FileKind::from_path(&path);
            spawn_local(async move {
                let Some(ws) = workspace_for.run(project_id) else {
                    return;
                };

                if kind == FileKind::Image
                    && let Ok(url) = ws.read_blob_url(&path).await
                {
                    if active_project.get_untracked() == Some(project_id)
                        && workspace.open_file.get_untracked().as_deref() == Some(path.as_str())
                    {
                        workspace.media_url.set(Some(url));
                    } else {
                        revoke_object_url(Some(url));
                    }
                }

                if kind.is_non_text() {
                    return;
                }

                match ws.read(&path).await {
                    Ok(content) => {
                        if active_project.get_untracked() == Some(project_id)
                            && workspace.open_file.get_untracked().as_deref() == Some(path.as_str())
                        {
                            workspace.content.set(content);
                        }
                    }
                    Err(error) => {
                        if active_project.get_untracked() == Some(project_id)
                            && workspace.open_file.get_untracked().as_deref() == Some(path.as_str())
                            && !error.contains("not valid UTF-8")
                        {
                            ui.toast.set(Some(error));
                        }
                    }
                }
            });
        });

        let request_open = Callback::new(move |path: String| {
            let current = workspace.open_file.get_untracked();
            if workspace.dirty.get_untracked() && current.as_deref() != Some(path.as_str()) {
                let current_path = current.unwrap_or_default();
                let path_clone = path.clone();
                ui.confirm.set(Some(ConfirmRequest {
                    title: "Discard unsaved changes".to_string(),
                    message: format!(
                        "`{current_path}` has unsaved changes. Discard them and open `{path}`?"
                    ),
                    confirm_label: "Discard".to_string(),
                    action: Callback::new(move |()| on_open.run(path_clone.clone())),
                }));
            } else {
                on_open.run(path);
            }
        });

        let on_toggle = Callback::new(move |dir: String| {
            let mut expanded = workspace.expanded.get();
            let opening = !expanded.contains(&dir);
            if opening {
                expanded.insert(dir.clone());
            } else {
                expanded.remove(&dir);
            }
            workspace.expanded.set(expanded);
            if opening
                && let Some(project_id) = active_project.get()
                && !workspace.entries.get().contains_key(&dir)
            {
                load_dir.run((project_id, dir));
            }
        });

        let on_save = Callback::new(move |()| {
            let Some(project_id) = active_project.get() else {
                return;
            };
            let Some(path) = workspace.open_file.get() else {
                return;
            };
            if !workspace.dirty.get() {
                return;
            }
            let content = workspace.content.get();
            ui.toast.set(None);
            spawn_local(async move {
                let Some(ws) = workspace_for.run(project_id) else {
                    return;
                };
                match ws.write(&path, &content).await {
                    Ok(()) => {
                        if active_project.get_untracked() == Some(project_id)
                            && workspace.open_file.get_untracked().as_deref() == Some(path.as_str())
                            && workspace.content.get_untracked() == content
                        {
                            workspace.dirty.set(false);
                        } else {
                            workspace.snapshots.update(|snapshots| {
                                if let Some(snapshot) = snapshots.get_mut(&project_id)
                                    && snapshot.open_file.as_deref() == Some(path.as_str())
                                    && snapshot.content == content
                                {
                                    snapshot.dirty = false;
                                }
                            });
                        }
                        refresh_git.run(());
                    }
                    Err(error) => ui.toast.set(Some(error)),
                }
            });
        });

        let on_accept = Callback::new(move |()| {
            let Some(project_id) = active_project.get() else {
                return;
            };
            let Some(path) = workspace.open_file.get() else {
                return;
            };
            let Some(diff) = workspace
                .pending_edits
                .with(|pending| pending.get(&path).cloned())
            else {
                return;
            };
            workspace.pending_edits.update(|pending| {
                pending.remove(&path);
            });

            if let Some(backup_path) = diff.backup_path {
                spawn_local(async move {
                    if let Some(ws) = workspace_for.run(project_id) {
                        let _ = ws.delete(&backup_path).await;
                    }
                });
            }

            workspace.content.set(diff.new);
            workspace.dirty.set(false);
            load_dir.run((project_id, parent_dir(&path)));
            refresh_git.run(());
        });

        let on_reject = Callback::new(move |()| {
            let Some(project_id) = active_project.get() else {
                return;
            };
            let Some(path) = workspace.open_file.get() else {
                return;
            };
            let Some(diff) = workspace
                .pending_edits
                .with(|pending| pending.get(&path).cloned())
            else {
                return;
            };
            let action = crate::pending::reject_action(&diff);
            let action_clone = action.clone();
            let load_dir = load_dir;
            ui.confirm.set(Some(ConfirmRequest {
                title: "Reject edit".to_string(),
                message: match &action {
                    crate::pending::RejectAction::Restore(_) => {
                        "Reject this edit? The file will be restored to its previous contents."
                            .to_string()
                    }
                    crate::pending::RejectAction::RestoreFromBackup(_) => {
                        "Reject this edit? The file will be restored from backup.".to_string()
                    }
                    crate::pending::RejectAction::Delete => {
                        "Reject this edit? The newly created file will be deleted.".to_string()
                    }
                    crate::pending::RejectAction::Unavailable => "Cannot reject this edit because the file's previous contents were too large to back up and no copy exists.".to_string(),
                },
                confirm_label: match &action {
                    crate::pending::RejectAction::Unavailable => "Ok".to_string(),
                    _ => "Reject".to_string(),
                },
                action: Callback::new(move |()| {
                    let action = action_clone.clone();
                    if matches!(action, crate::pending::RejectAction::Unavailable) {
                        return;
                    }
                    let path = path.clone();
                    workspace.pending_edits.update(|pending| {
                        pending.remove(&path);
                    });
                    ui.toast.set(None);
                    spawn_local(async move {
                        let Some(ws) = workspace_for.run(project_id) else {
                            return;
                        };
                        let result = match &action {
                            crate::pending::RejectAction::Restore(previous) => {
                                ws.write(&path, previous)
                                    .await
                                    .map(|()| previous.clone())
                            }
                            crate::pending::RejectAction::RestoreFromBackup(backup) => {
                                let result = ws.copy(backup, &path).await;
                                if result.is_ok() {
                                    let _ = ws.delete(backup).await;
                                }
                                result.map(|()| String::new())
                            }
                            crate::pending::RejectAction::Delete => {
                                ws.delete(&path).await.map(|()| String::new())
                            }
                            crate::pending::RejectAction::Unavailable => unreachable!(),
                        };
                        match result {
                            Ok(content) => {
                                if active_project.get() == Some(project_id) {
                                    if matches!(action, crate::pending::RejectAction::Delete) {
                                        workspace.open_file.set(None);
                                    } else if matches!(action, crate::pending::RejectAction::RestoreFromBackup(_)) {
                                        workspace.open_file.set(None);
                                        workspace.open_file.set(Some(path.clone()));
                                    } else {
                                        workspace.content.set(content);
                                    }
                                    workspace.dirty.set(false);
                                    load_dir.run((project_id, parent_dir(&path)));
                                    refresh_git.run(());
                                }
                            }
                            Err(error) => {
                                if active_project.get() == Some(project_id) {
                                    ui.toast.set(Some(error));
                                }
                            }
                        }
                    });
                }),
            }));
        });

        let on_new_file = Callback::new(move |()| {
            if active_project.get().is_none() {
                return;
            }
            ui.prompt.set(Some(PromptRequest {
                title: "New file".to_string(),
                value: String::new(),
                placeholder: "Path relative to project (e.g. src/main.rs)".to_string(),
                submit_label: "Create".to_string(),
                on_submit: Callback::new(move |path: String| {
                    let Some(project_id) = active_project.get() else {
                        return;
                    };
                    let path = path.trim().to_string();
                    if path.is_empty() {
                        return;
                    }
                    ui.toast.set(None);
                    let request_open = request_open;
                    let load_dir = load_dir;
                    spawn_local(async move {
                        let Some(ws) = workspace_for.run(project_id) else {
                            return;
                        };
                        match ws.create(&path, false).await {
                            Ok(()) => {
                                if active_project.get() == Some(project_id) {
                                    load_dir.run((project_id, parent_dir(&path)));
                                    request_open.run(path);
                                }
                            }
                            Err(error) => {
                                if active_project.get() == Some(project_id) {
                                    ui.toast.set(Some(error));
                                }
                            }
                        }
                    });
                }),
            }));
        });

        let on_new_dir = Callback::new(move |()| {
            if active_project.get().is_none() {
                return;
            }
            ui.prompt.set(Some(PromptRequest {
                title: "New folder".to_string(),
                value: String::new(),
                placeholder: "Path relative to project (e.g. src/utils)".to_string(),
                submit_label: "Create".to_string(),
                on_submit: Callback::new(move |path: String| {
                    let Some(project_id) = active_project.get() else {
                        return;
                    };
                    let path = path.trim().to_string();
                    if path.is_empty() {
                        return;
                    }
                    ui.toast.set(None);
                    let load_dir = load_dir;
                    spawn_local(async move {
                        let Some(ws) = workspace_for.run(project_id) else {
                            return;
                        };
                        match ws.create(&path, true).await {
                            Ok(()) => {
                                if active_project.get() == Some(project_id) {
                                    load_dir.run((project_id, parent_dir(&path)));
                                }
                            }
                            Err(error) => {
                                if active_project.get() == Some(project_id) {
                                    ui.toast.set(Some(error));
                                }
                            }
                        }
                    });
                }),
            }));
        });

        let search_gen = StoredValue::new(0u64);
        let on_search = Callback::new(move |(query, options): (String, SearchOptions)| {
            let this_generation = {
                search_gen.update_value(|generation| *generation += 1);
                search_gen.get_value()
            };
            let Some(project_id) = active_project.get() else {
                return;
            };
            spawn_local(async move {
                let Some(ws) = workspace_for.run(project_id) else {
                    return;
                };
                match ws.search_content(&query, "", options).await {
                    Ok(results) => {
                        if active_project.get() == Some(project_id)
                            && search_gen.get_value() == this_generation
                        {
                            workspace.search.set(Some(results));
                        }
                    }
                    Err(error) => {
                        if active_project.get() == Some(project_id)
                            && search_gen.get_value() == this_generation
                        {
                            ui.toast.set(Some(error));
                        }
                    }
                }
            });
        });

        let on_clear_search = Callback::new(move |()| {
            search_gen.update_value(|generation| *generation += 1);
            workspace.search.set(None);
        });

        Self {
            workspace_for,
            on_grant_access,
            ensure_root,
            on_open_lossy,
            request_open,
            on_toggle,
            on_save,
            on_accept,
            on_reject,
            on_new_file,
            on_new_dir,
            on_search,
            on_clear_search,
        }
    }
}

fn parent_dir(path: &str) -> String {
    path.rfind('/')
        .map(|index| path[..index].to_string())
        .unwrap_or_default()
}

fn revoke_object_url(url: Option<String>) {
    if let Some(url) = url {
        let _ = web_sys::Url::revoke_object_url(&url);
    }
}
