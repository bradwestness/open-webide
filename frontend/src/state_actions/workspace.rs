use crate::state::{
    auth::AuthState,
    projects::ProjectsState,
    ui::{ConfirmRequest, PromptRequest, UiState},
    workspace::WorkspaceState,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{
    EditDecision, FileKind, PersistedEdit, ResolveEditRequest, vfs::SearchOptions,
};

use crate::{backend::Api, local_fs, workspace::Workspace};

pub async fn refresh_pending(
    api: Api,
    projects: ProjectsState,
    workspace: WorkspaceState,
    ui: UiState,
    auth: AuthState,
    project_id: i64,
) {
    let auth_generation = auth.generation.get_untracked();
    let token = workspace.begin_pending_refresh(project_id);
    let result = api
        .with_value(Clone::clone)
        .list_pending_edits(project_id)
        .await;
    if auth.generation.try_get_untracked() != Some(auth_generation)
        || !workspace.pending_refresh_current(project_id, token)
        || projects
            .projects
            .try_with_untracked(|projects| projects.iter().any(|p| p.id == project_id))
            != Some(true)
    {
        return;
    }
    match result {
        Ok(edits) => workspace.set_persisted_edits(project_id, edits),
        Err(error) => ui.notify(error),
    }
}

pub fn pending_refresh(
    api: Api,
    projects: ProjectsState,
    workspace: WorkspaceState,
    ui: UiState,
) -> Callback<i64> {
    let auth = expect_context::<AuthState>();
    Callback::new(move |project_id| {
        let generation = auth.generation.get_untracked();
        spawn_local(async move {
            if auth.generation.try_get_untracked() == Some(generation) {
                refresh_pending(api, projects, workspace, ui, auth, project_id).await;
            }
        });
    })
}

async fn save_resolution(
    api: Api,
    project_id: i64,
    edit: &PersistedEdit,
    decision: EditDecision,
    current: impl Fn() -> bool,
) -> Result<PersistedEdit, String> {
    let request = ResolveEditRequest {
        path: edit.path.clone(),
        revision: edit.revision,
        decision,
    };
    let backend = api.with_value(Clone::clone);
    match backend.resolve_pending_edit(project_id, &request).await {
        Ok(edit) => Ok(edit),
        Err(error) => {
            if !current() {
                return Err(error);
            }
            backend.resolve_pending_edit(project_id, &request).await
        }
    }
}

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
    pub on_search_input: Callback<(String, SearchOptions)>,
    pub on_cancel_search: Callback<()>,
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
        let auth = expect_context::<AuthState>();
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

        let resolve = Callback::new(move |(edit, decision): (PersistedEdit, EditDecision)| {
            let project_id = edit.project_id;
            let path = edit.path.clone();
            let key = (project_id, path.clone());
            if workspace
                .resolving_edits
                .with_untracked(|edits| edits.contains(&key))
            {
                return;
            }
            workspace.resolving_edits.update(|edits| {
                edits.insert(key.clone());
            });
            let agent_writes_before = workspace
                .agent_writes
                .with_untracked(|writes| writes.get(&key).copied().unwrap_or_default());
            let editor_before = (
                workspace.open_file.get_untracked(),
                workspace.content.get_untracked(),
                workspace.dirty.get_untracked(),
            );
            let auth_generation = auth.generation.get_untracked();
            let epoch = workspace.pending_epoch.get_untracked();
            ui.clear_toast();
            spawn_local(async move {
                let current = || {
                    auth.generation.try_get_untracked() == Some(auth_generation)
                        && workspace.pending_epoch.try_get_untracked() == Some(epoch)
                        && projects.projects.try_with_untracked(|projects| {
                            projects.iter().any(|p| p.id == project_id)
                        }) == Some(true)
                };
                if !current() {
                    return;
                }
                let action = crate::pending::reject_action(&edit.diff);
                let result = async {
                    if decision == EditDecision::Rejected {
                        let ws = workspace_for.run(project_id).ok_or_else(|| {
                            "Folder not available. Re-open the project to pick it again."
                                .to_string()
                        })?;
                        // Check the revision before mutating files as well as when saving the decision.
                        let edits = api
                            .with_value(Clone::clone)
                            .list_pending_edits(project_id)
                            .await?;
                        if !current() {
                            return Err("Review was interrupted.".into());
                        }
                        if !edits
                            .iter()
                            .any(|record| record.path == path && record.revision == edit.revision)
                        {
                            // A previous save may have committed even if its response was lost.
                            save_resolution(api, project_id, &edit, decision, current).await?;
                            return Ok(());
                        }
                        match &action {
                            crate::pending::RejectAction::Restore(previous) => {
                                ws.write(&path, previous).await?;
                            }
                            crate::pending::RejectAction::RestoreFromBackup(backup) => {
                                ws.copy(backup, &path).await?;
                            }
                            crate::pending::RejectAction::Delete => {
                                // A retry after a successful delete may find the file already absent.
                                let parent = parent_dir(&path);
                                let name = path.rsplit('/').next().unwrap_or(&path);
                                let entries = ws.list(&parent).await?;
                                if !current() {
                                    return Err("Review was interrupted.".into());
                                }
                                if entries.iter().any(|entry| entry.name == name) {
                                    ws.delete(&path).await?;
                                }
                            }
                            crate::pending::RejectAction::Unavailable => {
                                return Err("Original contents are unavailable.".into());
                            }
                        }
                    }
                    if !current() {
                        return Err("Review was interrupted.".into());
                    }
                    save_resolution(api, project_id, &edit, decision, current).await?;
                    Ok::<_, String>(())
                }
                .await;
                if !current() {
                    return;
                }
                if result.is_ok() {
                    // Finish the editor transition before clearing review exposes an editable buffer.
                    let transition = |snapshot: &mut crate::state::workspace::WorkspaceSnapshot| {
                        if snapshot.open_file.as_deref() != Some(path.as_str())
                            || snapshot.open_file != editor_before.0
                            || snapshot.content != editor_before.1
                            || snapshot.dirty != editor_before.2
                            || workspace.agent_writes.with_untracked(|writes| {
                                writes.get(&key).copied().unwrap_or_default()
                            }) != agent_writes_before
                            || snapshot
                                .persisted_edits
                                .get(&path)
                                .is_some_and(|record| record.revision != edit.revision)
                        {
                            return false;
                        }
                        if decision == EditDecision::Accepted {
                            snapshot.content.clone_from(&edit.diff.new);
                        } else {
                            match &action {
                                crate::pending::RejectAction::Restore(previous) => {
                                    snapshot.content.clone_from(previous);
                                }
                                crate::pending::RejectAction::RestoreFromBackup(_) => {}
                                crate::pending::RejectAction::Delete => snapshot.open_file = None,
                                crate::pending::RejectAction::Unavailable => unreachable!(),
                            }
                        }
                        snapshot.dirty = false;
                        true
                    };
                    if active_project.get_untracked() == Some(project_id) {
                        let mut snapshot = workspace.active_snapshot();
                        if transition(&mut snapshot) {
                            workspace.content.set(snapshot.content);
                            if decision == EditDecision::Rejected
                                && matches!(
                                    action,
                                    crate::pending::RejectAction::RestoreFromBackup(_)
                                )
                            {
                                workspace.open_file.set(None);
                            }
                            workspace.open_file.set(snapshot.open_file);
                            workspace.dirty.set(snapshot.dirty);
                            load_dir.run((project_id, parent_dir(&path)));
                            refresh_git.run(());
                        }
                    } else {
                        workspace.snapshots.update(|snapshots| {
                            if let Some(snapshot) = snapshots.get_mut(&project_id) {
                                transition(snapshot);
                            }
                        });
                    }
                    workspace.clear_resolved_edit(&edit);
                }
                // Invalidate older list responses and verify authoritative state after either outcome.
                refresh_pending(api, projects, workspace, ui, auth, project_id).await;
                if !current() {
                    return;
                }
                workspace.resolving_edits.update(|edits| {
                    edits.remove(&key);
                });
                match result {
                    Ok(()) => {
                        let still_pending = if active_project.get_untracked() == Some(project_id) {
                            workspace
                                .pending_edits
                                .with_untracked(|edits| edits.contains_key(&path))
                        } else {
                            workspace.snapshots.with_untracked(|snapshots| {
                                snapshots.get(&project_id).is_some_and(|snapshot| {
                                    snapshot.pending_edits.contains_key(&path)
                                })
                            })
                        };
                        if still_pending {
                            return;
                        }
                        if let Some(backup) = &edit.diff.backup_path
                            && let Some(ws) = workspace_for.run(project_id)
                        {
                            let _ = ws.delete(backup).await;
                        }
                    }
                    Err(error) => ui.notify(error),
                }
            });
        });

        let on_accept = Callback::new(move |()| {
            let Some(path) = workspace.open_file.get_untracked() else {
                return;
            };
            if let Some(edit) = workspace
                .persisted_edits
                .with_untracked(|edits| edits.get(&path).cloned())
            {
                resolve.run((edit, EditDecision::Accepted));
            }
        });

        let on_reject = Callback::new(move |()| {
            let Some(path) = workspace.open_file.get_untracked() else {
                return;
            };
            let Some(edit) = workspace
                .persisted_edits
                .with_untracked(|edits| edits.get(&path).cloned())
            else {
                return;
            };
            if workspace.is_resolving() {
                return;
            }
            let action = crate::pending::reject_action(&edit.diff);
            ui.confirm.set(Some(ConfirmRequest {
                title: "Reject edit".to_string(),
                message: match &action {
                    crate::pending::RejectAction::Restore(_) => "Reject this edit? The file will be restored to its previous contents.".to_string(),
                    crate::pending::RejectAction::RestoreFromBackup(_) => "Reject this edit? The file will be restored from backup.".to_string(),
                    crate::pending::RejectAction::Delete => "Reject this edit? The newly created file will be deleted.".to_string(),
                    crate::pending::RejectAction::Unavailable => "Cannot reject this edit because the file's previous contents were too large to back up and no copy exists.".to_string(),
                },
                confirm_label: if matches!(action, crate::pending::RejectAction::Unavailable) { "Ok" } else { "Reject" }.to_string(),
                action: Callback::new(move |()| {
                    if !matches!(action, crate::pending::RejectAction::Unavailable) {
                        resolve.run((edit.clone(), EditDecision::Rejected));
                    }
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
        let search_timer = StoredValue::new(None::<leptos::leptos_dom::helpers::TimeoutHandle>);
        let on_cancel_search = Callback::new(move |()| {
            search_gen.try_update_value(|generation| *generation += 1);
            search_timer.try_update_value(|timer| {
                if let Some(timer) = timer.take() {
                    timer.clear();
                }
            });
        });
        on_cleanup(move || on_cancel_search.run(()));
        StoredValue::new_local(RenderEffect::new(move |_| {
            active_project.get();
            on_cancel_search.run(());
        }));

        let dispatch_search = Callback::new(
            move |(project_id, query, options, generation): (i64, String, SearchOptions, u64)| {
                spawn_local(async move {
                    if active_project.try_get_untracked() != Some(Some(project_id))
                        || search_gen.try_get_value() != Some(generation)
                    {
                        return;
                    }
                    let Some(ws) = workspace_for.run(project_id) else {
                        return;
                    };
                    let result = ws.search_content(&query, "", options).await;
                    if active_project.try_get_untracked() != Some(Some(project_id))
                        || search_gen.try_get_value() != Some(generation)
                    {
                        return;
                    }
                    match result {
                        Ok(results) => workspace.search.set(Some(results)),
                        Err(error) => ui.toast.set(Some(error)),
                    }
                });
            },
        );
        let on_search = Callback::new(move |(query, options): (String, SearchOptions)| {
            on_cancel_search.run(());
            if let Some(project_id) = active_project.get_untracked() {
                dispatch_search.run((project_id, query, options, search_gen.get_value()));
            }
        });
        let on_search_input = Callback::new(move |(query, options): (String, SearchOptions)| {
            // Invalidate in-flight responses before the debounce interval starts.
            on_cancel_search.run(());
            if query.is_empty() {
                workspace.search.set(None);
                return;
            }
            let Some(project_id) = active_project.get_untracked() else {
                return;
            };
            let generation = search_gen.get_value();
            if let Ok(timer) = leptos::leptos_dom::helpers::set_timeout_with_handle(
                move || {
                    search_timer.set_value(None);
                    dispatch_search.run((project_id, query, options, generation));
                },
                std::time::Duration::from_millis(250),
            ) {
                search_timer.set_value(Some(timer));
            }
        });
        let on_clear_search = Callback::new(move |()| {
            on_cancel_search.run(());
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
            on_search_input,
            on_cancel_search,
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
