use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{
    ChatMessage, ChatSession, Connection, FileDiff, FileEntry, ModelInfo, Project, Role, SearchHit,
    SystemPrompt, User, WorkspaceMode,
};
use web_sys::{AbortController, FileSystemDirectoryHandle};

use crate::api::{BackendApi, HealthState, SseEvent};
use crate::components::{
    AuthGate, ChatPane, ConversationItem, Editor, FileTree, Settings, Sidebar, StatusBar, TabBar,
    ToolStepResult, TopBar,
};
use crate::idb;
use crate::local_fs;
use crate::workspace::Workspace;

/// Per-project workspace state, preserved across tab switches so each project
/// keeps its own open file, tree expansion, and active chat session.
#[derive(Clone, Default)]
struct ProjectWorkspace {
    entries: HashMap<String, Vec<FileEntry>>,
    expanded: HashSet<String>,
    open_file: Option<String>,
    content: String,
    dirty: bool,
    search: Option<Vec<SearchHit>>,
    active_session: Option<i64>,
    /// Pending agent edits awaiting accept/reject, keyed by file path.
    pending_edits: HashMap<String, FileDiff>,
}

/// The directory containing `path` ("" for a top-level path).
fn parent_dir(path: &str) -> String {
    path.rfind('/')
        .map(|i| path[..i].to_string())
        .unwrap_or_default()
}

/// The cached theme from localStorage, applied before the backend responds so
/// the correct theme shows without a flash. Defaults to dark.
fn read_theme_from_storage() -> String {
    let theme = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|ls| ls.get_item("owide-theme").ok().flatten());
    match theme.as_deref() {
        Some("light") => "light".to_string(),
        _ => "dark".to_string(),
    }
}

fn write_theme_to_storage(theme: &str) {
    if let Some(ls) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = ls.set_item("owide-theme", theme);
    }
}

#[component]
pub fn App() -> impl IntoView {
    let api = BackendApi::from_location();

    // -- auth --------------------------------------------------------------
    // The signed-in account, once the cached token has been verified (or the
    // user has logged in). `None` while the gate is showing.
    let current_user = RwSignal::new(Option::<User>::None);
    // True once the initial token check has finished (regardless of outcome).
    let auth_checked = RwSignal::new(false);
    // The signed-in user's name, for the top bar.
    let username = RwSignal::new(Option::<String>::None);

    // -- global state ------------------------------------------------------
    let health = RwSignal::new(Option::<HealthState>::None);
    let connections = RwSignal::new(Vec::<Connection>::new());
    let projects = RwSignal::new(Vec::<Project>::new());
    let open_tabs = RwSignal::new(Vec::<Project>::new());
    let active_project = RwSignal::new(Option::<i64>::None);
    let sessions = RwSignal::new(Vec::<ChatSession>::new());
    let active_session = RwSignal::new(Option::<i64>::None);
    let messages = RwSignal::new(Vec::<ConversationItem>::new());
    let streaming = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let draft = RwSignal::new(String::new());
    let abort = RwSignal::new(Option::<AbortController>::None);
    // Models reported by the active session's connection, and the model each
    // session has chosen (None = the connection's default).
    let models = RwSignal::new(Vec::<ModelInfo>::new());
    let session_model = RwSignal::new(HashMap::<i64, Option<String>>::new());

    // -- active project's workspace state ----------------------------------
    let ws_entries = RwSignal::new(HashMap::<String, Vec<FileEntry>>::new());
    let ws_expanded = RwSignal::new(HashSet::<String>::new());
    let ws_open_file = RwSignal::new(Option::<String>::None);
    let ws_content = RwSignal::new(String::new());
    let ws_dirty = RwSignal::new(false);
    let ws_search = RwSignal::new(Option::<Vec<SearchHit>>::None);
    let ws_pending_edits = RwSignal::new(HashMap::<String, FileDiff>::new());
    // The pending edit for the currently open file (drives the editor's diff
    // view), derived from the open file and the per-project pending edits.
    let pending_diff = RwSignal::new(Option::<FileDiff>::None);
    // Saved workspace state for every project that has (or had) a tab open.
    let saved = RwSignal::new(HashMap::<i64, ProjectWorkspace>::new());
    // Directory handles for local-mode projects, keyed by project id. Loaded
    // from IndexedDB on startup and when a local project is created.
    let local_handles = RwSignal::new(HashMap::<i64, FileSystemDirectoryHandle>::new());

    // -- new-project form state --------------------------------------------
    let show_new_project = RwSignal::new(false);
    let np_name = RwSignal::new(String::new());
    let np_mode = RwSignal::new(WorkspaceMode::Remote);
    let np_path = RwSignal::new(String::new());

    // -- system prompts ----------------------------------------------------
    let system_prompts = RwSignal::new(Vec::<SystemPrompt>::new());
    // The create/edit form. `prompt_edit_id` is None when creating, Some(id)
    // when editing an existing prompt.
    let show_prompt_form = RwSignal::new(false);
    let prompt_edit_id = RwSignal::new(Option::<i64>::None);
    let prompt_name = RwSignal::new(String::new());
    let prompt_content = RwSignal::new(String::new());

    // -- settings ----------------------------------------------------------
    let show_settings = RwSignal::new(false);
    let theme = RwSignal::new(read_theme_from_storage());
    // Defaults applied to new sessions (None = the connection's own default).
    let default_connection = RwSignal::new(Option::<i64>::None);
    let default_prompt = RwSignal::new(Option::<i64>::None);

    // -- helpers -----------------------------------------------------------

    // A successful login / registration: record the account. The main
    // initial-load effect keys off `current_user`, so the user's data loads
    // as soon as this is set.
    let on_authed = Callback::new(move |user: User| {
        current_user.set(Some(user));
    });

    // Log out: discard the token and reset all per-user state so the gate
    // reappears with a clean slate.
    let on_logout = {
        let api = api.clone();
        Callback::new(move |_| {
            api.set_token(None);
            current_user.set(None);
            open_tabs.set(Vec::new());
            active_project.set(None);
            active_session.set(None);
            projects.set(Vec::new());
            sessions.set(Vec::new());
            messages.set(Vec::new());
            ws_entries.set(HashMap::new());
            ws_expanded.set(HashSet::new());
            ws_open_file.set(None);
            ws_content.set(String::new());
            ws_dirty.set(false);
            ws_search.set(None);
            ws_pending_edits.set(HashMap::new());
            saved.set(HashMap::new());
            local_handles.set(HashMap::new());
            error.set(None);
        })
    };

    // `Copy` handles so the auth-gate fallback (a `view!` inside a closure) can
    // hand the API and the auth callback to the gate without moving them out of
    // the closure, which would make the fallback `FnOnce`.
    let api_ref = RwSignal::new(api.clone());
    let on_authed_ref = RwSignal::new(on_authed);

    // Build the workspace for a project based on its mode. Remote wraps the
    // backend API; local wraps the project's directory handle (if loaded).
    let ws_api = api.clone();
    let workspace_for = Callback::new(move |pid: i64| -> Option<Workspace> {
        let project = projects.get().into_iter().find(|p| p.id == pid)?;
        match project.mode {
            WorkspaceMode::Remote => Some(Workspace::Remote {
                api: ws_api.clone(),
                project_id: pid,
            }),
            WorkspaceMode::Local => local_handles
                .get()
                .get(&pid)
                .cloned()
                .map(|handle| Workspace::Local { handle }),
        }
    });

    // Load a directory's entries into the cache (overwriting any stale copy).
    let load_dir = Callback::new(move |(id, dir): (i64, String)| {
        spawn_local(async move {
            let Some(ws) = workspace_for.run(id) else {
                // A local project whose directory handle is missing (e.g. the
                // browser permission was not granted after a reload). Surface
                // it instead of leaving the tree silently empty.
                if active_project.get() == Some(id) {
                    error.set(Some(
                        "Folder not available. Re-open the project to pick it again.".to_string(),
                    ));
                }
                return;
            };
            match ws.list(&dir).await {
                Ok(entries) => {
                    if active_project.get() == Some(id) {
                        ws_entries.update(|m| {
                            m.insert(dir, entries);
                        });
                    }
                }
                Err(e) => {
                    if active_project.get() == Some(id) {
                        error.set(Some(e));
                    }
                }
            }
        });
    });

    // Make sure the active project's root directory is loaded.
    let ensure_root = Callback::new(move |id: i64| {
        if ws_entries.get().contains_key("") {
            return;
        }
        load_dir.run((id, String::new()));
    });

    // Open a file: clear the dirty flag and load its contents.
    let on_open = Callback::new(move |path: String| {
        let Some(pid) = active_project.get() else {
            return;
        };
        ws_dirty.set(false);
        ws_open_file.set(Some(path.clone()));
        ws_content.set(String::new());
        error.set(None);
        spawn_local(async move {
            let Some(ws) = workspace_for.run(pid) else {
                return;
            };
            match ws.read(&path).await {
                Ok(content) => {
                    if ws_open_file.get().as_deref() == Some(&path) {
                        ws_content.set(content);
                    }
                }
                Err(e) => {
                    if ws_open_file.get().as_deref() == Some(&path) {
                        error.set(Some(e));
                    }
                }
            }
        });
    });

    // Toggle a directory's expansion, lazily loading its children on first open.
    let on_toggle = Callback::new(move |dir: String| {
        let mut ex = ws_expanded.get();
        let opening = !ex.contains(&dir);
        if opening {
            ex.insert(dir.clone());
        } else {
            ex.remove(&dir);
        }
        ws_expanded.set(ex);
        if opening
            && let Some(pid) = active_project.get()
            && !ws_entries.get().contains_key(&dir)
        {
            load_dir.run((pid, dir));
        }
    });

    // Save the open file's contents to disk.
    let on_save = Callback::new(move |_| {
        let Some(pid) = active_project.get() else {
            return;
        };
        let Some(path) = ws_open_file.get() else {
            return;
        };
        if !ws_dirty.get() {
            return;
        }
        let content = ws_content.get();
        error.set(None);
        spawn_local(async move {
            let Some(ws) = workspace_for.run(pid) else {
                return;
            };
            match ws.write(&path, &content).await {
                Ok(()) => ws_dirty.set(false),
                Err(e) => error.set(Some(e)),
            }
        });
    });

    // Accept a pending agent edit: the file is already on disk with the new
    // contents, so just clear the pending entry, show the new contents, and
    // refresh the tree (so a newly created file appears).
    let on_accept = Callback::new(move |_| {
        let Some(pid) = active_project.get() else {
            return;
        };
        let Some(path) = ws_open_file.get() else {
            return;
        };
        let Some(diff) = ws_pending_edits.with(|m| m.get(&path).cloned()) else {
            return;
        };
        ws_pending_edits.update(|m| {
            m.remove(&path);
        });
        ws_content.set(diff.new);
        ws_dirty.set(false);
        load_dir.run((pid, parent_dir(&path)));
    });

    // Reject a pending agent edit: restore the previous contents (or delete a
    // newly created file), then refresh the tree.
    let on_reject = Callback::new(move |_| {
        let Some(pid) = active_project.get() else {
            return;
        };
        let Some(path) = ws_open_file.get() else {
            return;
        };
        let Some(diff) = ws_pending_edits.with(|m| m.get(&path).cloned()) else {
            return;
        };
        ws_pending_edits.update(|m| {
            m.remove(&path);
        });
        error.set(None);
        let is_new = diff.old.is_none();
        let prev = diff.old;
        let ld = load_dir;
        spawn_local(async move {
            let Some(ws) = workspace_for.run(pid) else {
                return;
            };
            let result = match prev {
                Some(prev) => {
                    let r = ws.write(&path, &prev).await;
                    r.map(|()| prev)
                }
                None => ws.delete(&path).await.map(|_| String::new()),
            };
            match result {
                Ok(content) => {
                    if active_project.get() == Some(pid) {
                        if is_new {
                            ws_open_file.set(None);
                        }
                        ws_content.set(content);
                        ws_dirty.set(false);
                        ld.run((pid, parent_dir(&path)));
                    }
                }
                Err(e) => {
                    if active_project.get() == Some(pid) {
                        error.set(Some(e));
                    }
                }
            }
        });
    });

    // Create a new empty file and open it.
    let on_new_file = Callback::new(move |_| {
        let Some(pid) = active_project.get() else {
            return;
        };
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(Some(prompt)) = window
            .prompt_with_message_and_default("New file path (relative to project):", "new.txt")
        else {
            return;
        };
        let path = prompt.trim().to_string();
        if path.is_empty() {
            return;
        }
        error.set(None);
        let open = on_open;
        let ld = load_dir;
        spawn_local(async move {
            let Some(ws) = workspace_for.run(pid) else {
                return;
            };
            match ws.create(&path, false).await {
                Ok(()) => {
                    if active_project.get() == Some(pid) {
                        ld.run((pid, parent_dir(&path)));
                        open.run(path);
                    }
                }
                Err(e) => {
                    if active_project.get() == Some(pid) {
                        error.set(Some(e));
                    }
                }
            }
        });
    });

    // Create a new directory and refresh its parent.
    let on_new_dir = Callback::new(move |_| {
        let Some(pid) = active_project.get() else {
            return;
        };
        let Some(window) = web_sys::window() else {
            return;
        };
        let Ok(Some(prompt)) =
            window.prompt_with_message_and_default("New folder path:", "new-folder")
        else {
            return;
        };
        let path = prompt.trim().to_string();
        if path.is_empty() {
            return;
        }
        error.set(None);
        let ld = load_dir;
        spawn_local(async move {
            let Some(ws) = workspace_for.run(pid) else {
                return;
            };
            match ws.create(&path, true).await {
                Ok(()) => {
                    if active_project.get() == Some(pid) {
                        ld.run((pid, parent_dir(&path)));
                    }
                }
                Err(e) => {
                    if active_project.get() == Some(pid) {
                        error.set(Some(e));
                    }
                }
            }
        });
    });

    // Full-text search the project's file contents.
    let on_search = Callback::new(move |q: String| {
        let Some(pid) = active_project.get() else {
            return;
        };
        spawn_local(async move {
            let Some(ws) = workspace_for.run(pid) else {
                return;
            };
            match ws.search_content(&q, "").await {
                Ok(results) => {
                    if active_project.get() == Some(pid) {
                        ws_search.set(Some(results));
                    }
                }
                Err(e) => {
                    if active_project.get() == Some(pid) {
                        error.set(Some(e));
                    }
                }
            }
        });
    });

    let on_clear_search = Callback::new(move |_| {
        ws_search.set(None);
    });

    // Switch the active tab: snapshot the current project's workspace, then
    // restore the target project's (or start fresh).
    let select_project = Callback::new(move |id: i64| {
        let cur = active_project.get();
        if cur == Some(id) {
            return;
        }
        if let Some(old) = cur {
            let ws = ProjectWorkspace {
                entries: ws_entries.get(),
                expanded: ws_expanded.get(),
                open_file: ws_open_file.get(),
                content: ws_content.get(),
                dirty: ws_dirty.get(),
                search: ws_search.get(),
                active_session: active_session.get(),
                pending_edits: ws_pending_edits.get(),
            };
            saved.update(|m| {
                m.insert(old, ws);
            });
        }
        active_project.set(Some(id));
        let ws = saved.with(|m| m.get(&id).cloned()).unwrap_or_default();
        ws_entries.set(ws.entries);
        ws_expanded.set(ws.expanded);
        ws_open_file.set(ws.open_file);
        ws_content.set(ws.content);
        ws_dirty.set(ws.dirty);
        ws_search.set(ws.search);
        ws_pending_edits.set(ws.pending_edits);
        active_session.set(ws.active_session);
        error.set(None);
        ensure_root.run(id);
    });

    // Close a project tab; if it was active, switch to another open tab.
    let close_project = Callback::new(move |id: i64| {
        open_tabs.update(|tabs| tabs.retain(|p| p.id != id));
        if active_project.get() == Some(id) {
            let next = open_tabs.get().into_iter().next().map(|p| p.id);
            match next {
                Some(nid) => select_project.run(nid),
                None => {
                    active_project.set(None);
                    active_session.set(None);
                    ws_entries.set(HashMap::new());
                    ws_expanded.set(HashSet::new());
                    ws_open_file.set(None);
                    ws_content.set(String::new());
                    ws_dirty.set(false);
                    ws_search.set(None);
                    ws_pending_edits.set(HashMap::new());
                }
            }
        }
    });

    // Open an existing project (from the list) as a tab.
    let on_open_project = Callback::new(move |id: i64| {
        open_tabs.update(|tabs| {
            if !tabs.iter().any(|t| t.id == id)
                && let Some(p) = projects.get().into_iter().find(|p| p.id == id)
            {
                tabs.push(p);
            }
        });
        select_project.run(id);
    });

    // Show the new-project form, resetting its fields.
    let on_new_project = Callback::new(move |_| {
        show_new_project.set(true);
        np_name.set(String::new());
        np_mode.set(WorkspaceMode::Remote);
        np_path.set(String::new());
    });

    let on_cancel_new = Callback::new(move |_| {
        show_new_project.set(false);
    });

    // Create a project (open a folder) and open it as a tab.
    let on_create_project = {
        let api = api.clone();
        Callback::new(move |_| {
            let mode = np_mode.get();
            error.set(None);
            let api = api.clone();
            spawn_local(async move {
                // Local mode: pick a directory in the browser, persist its
                // handle, then create the project record. The picked folder's
                // name is the project name, so no name is asked for up front.
                if mode == WorkspaceMode::Local {
                    let picked = match local_fs::pick_directory().await {
                        Ok(picked) => picked,
                        Err(e) => {
                            error.set(Some(e));
                            return;
                        }
                    };
                    let Some(handle) = picked else {
                        // The user dismissed the picker; nothing to do.
                        return;
                    };
                    let name = handle.name();
                    let project = match api.create_project(&name, mode, Some(name.clone())).await {
                        Ok(project) => project,
                        Err(e) => {
                            error.set(Some(e));
                            return;
                        }
                    };
                    if let Err(e) = idb::save_handle(project.id, &handle).await {
                        let _ = api.delete_project(project.id).await;
                        error.set(Some(e));
                        return;
                    }
                    local_handles.update(|m| {
                        m.insert(project.id, handle);
                    });
                    projects.update(|all| all.push(project.clone()));
                    open_tabs.update(|tabs| tabs.push(project.clone()));
                    show_new_project.set(false);
                    select_project.run(project.id);
                    return;
                }
                // Remote mode: the path is relative to the mounted host folder
                // (see spin.toml). Empty means the root of the mount.
                let name = np_name.get().trim().to_string();
                if name.is_empty() {
                    error.set(Some("Project name is required.".to_string()));
                    return;
                }
                let path = Some(np_path.get().trim().to_string());
                match api.create_project(&name, mode, path).await {
                    Ok(p) => {
                        projects.update(|all| all.push(p.clone()));
                        open_tabs.update(|tabs| tabs.push(p.clone()));
                        show_new_project.set(false);
                        select_project.run(p.id);
                    }
                    Err(e) => error.set(Some(e)),
                }
            });
        })
    };

    // -- session callbacks -------------------------------------------------

    let on_select_session = Callback::new(move |id: i64| {
        active_session.set(Some(id));
    });

    let on_new_session = {
        let api = api.clone();
        Callback::new(move |_| {
            let Some(pid) = active_project.get() else {
                return;
            };
            let name = format!("Session {}", sessions.get().len() + 1);
            let api = api.clone();
            let conn = default_connection.get();
            let prompt = default_prompt.get();
            spawn_local(async move {
                match api.create_session(&name, conn, prompt, Some(pid)).await {
                    Ok(s) => {
                        sessions.update(|all| all.push(s.clone()));
                        active_session.set(Some(s.id));
                        error.set(None);
                    }
                    Err(e) => error.set(Some(e)),
                }
            });
        })
    };

    let on_rename_session = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let Some(window) = web_sys::window() else {
                return;
            };
            let current = sessions
                .with(|list| list.iter().find(|s| s.id == id).map(|s| s.name.clone()))
                .unwrap_or_default();
            let Ok(Some(name)) = window.prompt_with_message_and_default("Rename session", &current)
            else {
                return;
            };
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            let api = api.clone();
            spawn_local(async move {
                match api.rename_session(id, &name).await {
                    Ok(updated) => sessions.update(|list| {
                        if let Some(s) = list.iter_mut().find(|s| s.id == id) {
                            *s = updated;
                        }
                    }),
                    Err(e) => error.set(Some(e)),
                }
            });
        })
    };

    let on_delete_session = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let Some(window) = web_sys::window() else {
                return;
            };
            let Ok(confirmed) =
                window.confirm_with_message("Delete this session and its messages?")
            else {
                return;
            };
            if !confirmed {
                return;
            }
            let api = api.clone();
            spawn_local(async move {
                if let Err(e) = api.delete_session(id).await {
                    error.set(Some(e));
                    return;
                }
                sessions.update(|list| list.retain(|s| s.id != id));
                if active_session.get() == Some(id) {
                    active_session.set(None);
                }
            });
        })
    };

    // Delete a project: remove it (and its sessions) from the backend, drop
    // its cached workspace state and local directory handle, then close its
    // tab (switching the active project if it was open).
    let on_delete_project = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let Some(window) = web_sys::window() else {
                return;
            };
            let Ok(confirmed) = window.confirm_with_message(
                "Delete this project? Its sessions and messages will be removed too.",
            ) else {
                return;
            };
            if !confirmed {
                return;
            }
            let api = api.clone();
            spawn_local(async move {
                if let Err(e) = api.delete_project(id).await {
                    error.set(Some(e));
                    return;
                }
                saved.update(|m| {
                    m.remove(&id);
                });
                local_handles.update(|m| {
                    m.remove(&id);
                });
                projects.update(|all| all.retain(|p| p.id != id));
                sessions.update(|list| list.retain(|s| s.project_id != Some(id)));
                close_project.run(id);
            });
        })
    };

    // -- system prompt callbacks -------------------------------------------

    // Show the form to create a new prompt.
    let on_new_prompt = Callback::new(move |_| {
        show_prompt_form.set(true);
        prompt_edit_id.set(None);
        prompt_name.set(String::new());
        prompt_content.set(String::new());
    });

    // Show the form to edit an existing prompt, loading its current values.
    let on_edit_prompt = Callback::new(move |id: i64| {
        let Some(p) = system_prompts.get().into_iter().find(|p| p.id == id) else {
            return;
        };
        prompt_edit_id.set(Some(id));
        prompt_name.set(p.name);
        prompt_content.set(p.content);
        show_prompt_form.set(true);
    });

    let on_cancel_prompt = Callback::new(move |_| {
        show_prompt_form.set(false);
    });

    // Create or update the prompt, then refresh the list.
    let on_save_prompt = {
        let api = api.clone();
        Callback::new(move |_| {
            let name = prompt_name.get().trim().to_string();
            if name.is_empty() {
                error.set(Some("Prompt name is required.".to_string()));
                return;
            }
            let content = prompt_content.get();
            let edit_id = prompt_edit_id.get();
            error.set(None);
            let api = api.clone();
            spawn_local(async move {
                let result = match edit_id {
                    Some(id) => api.update_system_prompt(id, &name, &content).await,
                    None => api.create_system_prompt(&name, &content).await,
                };
                match result {
                    Ok(p) => {
                        system_prompts.update(|list| match edit_id {
                            Some(id) => {
                                if let Some(existing) = list.iter_mut().find(|x| x.id == id) {
                                    *existing = p;
                                }
                            }
                            None => list.push(p),
                        });
                        show_prompt_form.set(false);
                    }
                    Err(e) => error.set(Some(e)),
                }
            });
        })
    };

    let on_delete_prompt = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let Some(window) = web_sys::window() else {
                return;
            };
            let Ok(confirmed) = window.confirm_with_message("Delete this system prompt?") else {
                return;
            };
            if !confirmed {
                return;
            }
            let api = api.clone();
            spawn_local(async move {
                if let Err(e) = api.delete_system_prompt(id).await {
                    error.set(Some(e));
                    return;
                }
                system_prompts.update(|list| list.retain(|p| p.id != id));
            });
        })
    };

    // -- settings callbacks ------------------------------------------------

    let on_open_settings = Callback::new(move |_| {
        show_settings.set(true);
    });

    let on_close_settings = Callback::new(move |_| {
        show_settings.set(false);
    });

    let on_set_theme = {
        let api = api.clone();
        Callback::new(move |value: String| {
            theme.set(value.clone());
            write_theme_to_storage(&value);
            let api = api.clone();
            spawn_local(async move {
                if let Err(e) = api.set_setting("theme", &value).await {
                    error.set(Some(e));
                }
            });
        })
    };

    let on_set_default_connection = {
        let api = api.clone();
        Callback::new(move |value: Option<i64>| {
            default_connection.set(value);
            let v = value.map(|id| id.to_string()).unwrap_or_default();
            let api = api.clone();
            spawn_local(async move {
                if let Err(e) = api.set_setting("default_connection", &v).await {
                    error.set(Some(e));
                }
            });
        })
    };

    let on_set_default_prompt = {
        let api = api.clone();
        Callback::new(move |value: Option<i64>| {
            default_prompt.set(value);
            let v = value.map(|id| id.to_string()).unwrap_or_default();
            let api = api.clone();
            spawn_local(async move {
                if let Err(e) = api.set_setting("default_prompt", &v).await {
                    error.set(Some(e));
                }
            });
        })
    };

    // -- send / stop -------------------------------------------------------

    let on_send = {
        let api = api.clone();
        Callback::new(move |_| {
            let content = draft.with(|d| d.trim().to_string());
            if content.is_empty() || streaming.get() {
                return;
            }
            let Some(session_id) = active_session.get() else {
                return;
            };
            let model = session_model.get().get(&session_id).cloned().flatten();
            let Ok(controller) = AbortController::new() else {
                return;
            };
            draft.set(String::new());
            streaming.set(true);
            error.set(None);
            abort.set(Some(controller.clone()));
            let api = api.clone();
            spawn_local(async move {
                let result = api
                    .send_message(
                        session_id,
                        &content,
                        model.as_deref(),
                        Some(&controller.signal()),
                        move |event| {
                            if active_session.get() != Some(session_id) {
                                return;
                            }
                            match event {
                                SseEvent::Message(msg) => {
                                    messages.update(|m| m.push(ConversationItem::Message(msg)));
                                }
                                SseEvent::Delta(delta) => {
                                    messages.update(|m| {
                                        let extend_last = m
                                            .last()
                                            .is_some_and(|last| {
                                                matches!(
                                                    last,
                                                    ConversationItem::Message(msg)
                                                        if msg.role == Role::Assistant
                                                )
                                            });
                                        if extend_last {
                                            if let Some(ConversationItem::Message(msg)) = m.last_mut() {
                                                msg.content.push_str(&delta);
                                            }
                                        } else {
                                            m.push(ConversationItem::Message(ChatMessage {
                                                id: 0,
                                                session_id,
                                                role: Role::Assistant,
                                                content: delta,
                                                created_at: 0,
                                                tool_calls: None,
                                                tool_call_id: None,
                                            }));
                                        }
                                    });
                                }
                                SseEvent::ToolCall { id, name, summary } => {
                                    messages.update(|m| {
                                        m.push(ConversationItem::ToolStep {
                                            id,
                                            name,
                                            summary,
                                            result: None,
                                        })
                                    });
                                }
                                SseEvent::ToolResult { id, name, ok, summary, diff } => {
                                    // Record the agent's edit as a pending diff
                                    // and open the file so the editor shows it
                                    // in diff mode.
                                    if let Some(d) = &diff {
                                        ws_pending_edits.update(|m| {
                                            m.insert(d.path.clone(), d.clone());
                                        });
                                        if ws_open_file.get().as_deref() != Some(d.path.as_str()) {
                                            ws_open_file.set(Some(d.path.clone()));
                                            ws_dirty.set(false);
                                        }
                                    }
                                    messages.update(|m| {
                                        let idx = m.iter().rposition(|item| {
                                            matches!(
                                                item,
                                                ConversationItem::ToolStep { id: tid, .. } if *tid == id
                                            )
                                        });
                                        match idx {
                                            Some(i) => {
                                                if let ConversationItem::ToolStep {
                                                    result: r, ..
                                                } = &mut m[i]
                                                {
                                                    *r = Some(ToolStepResult {
                                                        ok,
                                                        summary: summary.clone(),
                                                        diff: diff.clone(),
                                                    });
                                                }
                                            }
                                            None => {
                                                m.push(ConversationItem::ToolStep {
                                                    id,
                                                    name,
                                                    summary: summary.clone(),
                                                    result: Some(ToolStepResult { ok, summary, diff }),
                                                });
                                            }
                                        }
                                    });
                                }
                                SseEvent::Done(msg) => {
                                    messages.update(|m| {
                                        if let Some(ConversationItem::Message(last)) = m.last_mut()
                                            && last.role == Role::Assistant
                                        {
                                            *last = msg;
                                        }
                                    });
                                }
                                SseEvent::Error(e) => error.set(Some(e)),
                            }
                        },
                    )
                    .await;
                if let Err(e) = result
                    && !controller.signal().aborted()
                {
                    error.set(Some(e));
                }
                streaming.set(false);
                abort.set(None);
            });
        })
    };

    let on_stop = Callback::new(move |_| {
        abort.with(|a| {
            if let Some(c) = a.as_ref() {
                c.abort();
            }
        });
    });

    // -- effects -----------------------------------------------------------

    // Apply the theme to the document root so the CSS variables switch.
    Effect::new(move || {
        let t = theme.get();
        if let Some(doc) = web_sys::window().and_then(|w| w.document())
            && let Some(root) = doc.document_element()
        {
            let _ = root.set_attribute("data-theme", &t);
        }
    });

    // One-shot auth check: if a token is cached, verify it with /me. A
    // failure (or no token) leaves us unauthenticated, so the gate shows.
    {
        let api = api.clone();
        Effect::new(move || {
            if auth_checked.get() {
                return;
            }
            let api = api.clone();
            spawn_local(async move {
                let user = if api.token().is_some() {
                    api.me().await.ok()
                } else {
                    None
                };
                current_user.set(user);
                auth_checked.set(true);
            });
        });
    }

    // The signed-in user's name for the top bar.
    Effect::new(move || {
        username.set(current_user.get().map(|u| u.username));
    });

    // Initial load: health, connections, projects, sessions, then auto-open
    // the first project as a tab. Runs once the user is authenticated.
    {
        let api = api.clone();
        let sel = select_project;
        Effect::new(move || {
            if current_user.get().is_none() {
                return;
            }
            let api = api.clone();
            spawn_local(async move {
                let backend_ok = match api.health().await {
                    Ok(h) => {
                        health.set(Some(HealthState::Online { version: h.version }));
                        true
                    }
                    Err(_) => {
                        health.set(Some(HealthState::Offline));
                        false
                    }
                };
                if !backend_ok {
                    return;
                }
                if let Ok(conns) = api.list_connections().await {
                    connections.set(conns);
                }
                if let Ok(p) = api.list_projects().await {
                    projects.set(p);
                }
                // Load persisted directory handles for local-mode projects so
                // they can be browsed after a page reload.
                for project in projects
                    .get()
                    .into_iter()
                    .filter(|p| p.mode == WorkspaceMode::Local)
                {
                    if let Ok(Some(handle)) = idb::load_handle(project.id).await {
                        let _ = idb::request_permission(&handle).await;
                        local_handles.update(|m| {
                            m.insert(project.id, handle);
                        });
                    }
                }
                if let Ok(s) = api.list_sessions().await {
                    sessions.set(s);
                }
                if let Ok(prompts) = api.list_system_prompts().await {
                    system_prompts.set(prompts);
                }
                if let Ok(s) = api.get_settings().await {
                    if let Some(t) = s.get("theme").map(String::as_str)
                        && (t == "dark" || t == "light")
                    {
                        theme.set(t.to_string());
                    }
                    if let Some(v) = s
                        .get("default_connection")
                        .and_then(|v| v.parse::<i64>().ok())
                    {
                        default_connection.set(Some(v));
                    }
                    if let Some(v) = s.get("default_prompt").and_then(|v| v.parse::<i64>().ok()) {
                        default_prompt.set(Some(v));
                    }
                }
                if let Some(first) = projects.get().into_iter().next() {
                    open_tabs.update(|tabs| {
                        if !tabs.iter().any(|t| t.id == first.id) {
                            tabs.push(first.clone());
                        }
                    });
                    sel.run(first.id);
                }
            });
        });
    }

    // Load the message history whenever the active session changes.
    {
        let api = api.clone();
        Effect::new(move || {
            let id = active_session.get();
            let api = api.clone();
            spawn_local(async move {
                match id {
                    Some(id) => match api.list_messages(id).await {
                        Ok(m) => {
                            messages.set(m.into_iter().map(ConversationItem::Message).collect())
                        }
                        Err(e) => error.set(Some(e)),
                    },
                    None => messages.set(Vec::new()),
                }
            });
        });
    }

    // Load the models the active session's connection offers.
    {
        let api = api.clone();
        Effect::new(move || {
            let sid = active_session.get();
            let conn = sid.and_then(|id| {
                sessions
                    .get()
                    .into_iter()
                    .find(|s| s.id == id)
                    .and_then(|s| s.connection_id)
            });
            let api = api.clone();
            spawn_local(async move {
                let list = match conn {
                    Some(cid) => api.list_models(cid).await.unwrap_or_default(),
                    None => Vec::new(),
                };
                if active_session.get() == sid {
                    models.set(list);
                }
            });
        });
    }

    // The model chosen for the active session (drives the picker's value).
    let selected_model = RwSignal::new(Option::<String>::None);
    Effect::new(move || {
        let sel = active_session
            .get()
            .and_then(|id| session_model.get().get(&id).cloned())
            .flatten();
        selected_model.set(sel);
    });

    let on_select_model = Callback::new(move |model: Option<String>| {
        let Some(sid) = active_session.get() else {
            return;
        };
        session_model.update(|m| {
            m.insert(sid, model);
        });
    });

    // Derived values for the view.
    let has_session = RwSignal::new(false);
    Effect::new(move || {
        has_session.set(active_session.get().is_some());
    });

    // The pending edit for the currently open file, if any.
    Effect::new(move || {
        let open = ws_open_file.get();
        let pending = ws_pending_edits.get();
        let diff = open.as_ref().and_then(|p| pending.get(p).cloned());
        pending_diff.set(diff);
    });

    // Agentic file tools only run for remote (Spin-hosted) projects; the
    // backend can't reach a local-mode project's browser-side files.
    let local_mode = RwSignal::new(false);
    Effect::new(move || {
        let mode = active_project.get().and_then(|pid| {
            projects
                .get()
                .into_iter()
                .find(|p| p.id == pid)
                .map(|p| p.mode)
        });
        local_mode.set(mode == Some(WorkspaceMode::Local));
    });

    view! {
        <Show
            when=move || auth_checked.get() && current_user.get().is_some()
            fallback=move || {
                view! {
                    <Show
                        when=move || auth_checked.get()
                        fallback=move || {
                            view! {
                                <div class="auth-gate">
                                    <div class="auth-loading">"Loading…"</div>
                                </div>
                            }
                        }
                    >
                        <AuthGate api={api_ref.get()} on_authed={on_authed_ref.get()} />
                    </Show>
                }
            }
        >
            <div class="app">
                <TopBar
                    health=health.read_only()
                    username=username.read_only()
                    on_open_settings=on_open_settings
                    on_logout=on_logout
                />
                <TabBar
                open_tabs=open_tabs.read_only()
                active_project=active_project.read_only()
                on_new=on_new_project
                on_select=select_project
                on_close=close_project
            />
            <div class="app-body">
                <Sidebar
                    connections=connections.read_only()
                    projects=projects.read_only()
                    sessions=sessions.read_only()
                    active_project=active_project.read_only()
                    active_session=active_session.read_only()
                    show_new_project=show_new_project.read_only()
                    np_name=np_name.read_only()
                    set_np_name=np_name.write_only()
                    np_mode=np_mode.read_only()
                    set_np_mode=np_mode.write_only()
                    np_path=np_path.read_only()
                    set_np_path=np_path.write_only()
                    on_new_project=on_new_project
                    on_create_project=on_create_project
                    on_cancel_new=on_cancel_new
                    on_open_project=on_open_project
                    on_delete_project=on_delete_project
                    on_select_session=on_select_session
                    on_new_session=on_new_session
                    on_rename_session=on_rename_session
                    on_delete_session=on_delete_session
                    system_prompts=system_prompts.read_only()
                    show_prompt_form=show_prompt_form.read_only()
                    prompt_edit_id=prompt_edit_id.read_only()
                    prompt_name=prompt_name.read_only()
                    set_prompt_name=prompt_name.write_only()
                    prompt_content=prompt_content.read_only()
                    set_prompt_content=prompt_content.write_only()
                    on_new_prompt=on_new_prompt
                    on_edit_prompt=on_edit_prompt
                    on_save_prompt=on_save_prompt
                    on_cancel_prompt=on_cancel_prompt
                    on_delete_prompt=on_delete_prompt
                />
                <FileTree
                    entries=ws_entries.read_only()
                    expanded=ws_expanded.read_only()
                    open_file=ws_open_file.read_only()
                    search_results=ws_search.read_only()
                    error=error.read_only()
                    on_toggle=on_toggle
                    on_open=on_open
                    on_new_file=on_new_file
                    on_new_dir=on_new_dir
                    on_search=on_search
                    on_clear_search=on_clear_search
                />
                <Editor
                    open_file=ws_open_file.read_only()
                    content=ws_content.read_only()
                    set_content=ws_content.write_only()
                    dirty=ws_dirty.read_only()
                    set_dirty=ws_dirty.write_only()
                    pending_diff=pending_diff.read_only()
                    on_save=on_save
                    on_accept=on_accept
                    on_reject=on_reject
                    error=error.read_only()
                />
                <ChatPane
                    messages=messages.read_only()
                    streaming=streaming.read_only()
                    draft=draft.read_only()
                    set_draft=draft.write_only()
                    error=error.read_only()
                    has_session=has_session.read_only()
                    local_mode=local_mode.read_only()
                    models=models.read_only()
                    selected_model=selected_model.read_only()
                    on_select_model=on_select_model
                    on_send=on_send
                    on_stop=on_stop
                />
            </div>
            <StatusBar health=health.read_only() />
            <Show when=move || show_settings.get() fallback=|| ()>
                <Settings
                    theme=theme.read_only()
                    default_connection=default_connection.read_only()
                    default_prompt=default_prompt.read_only()
                    connections=connections.read_only()
                    system_prompts=system_prompts.read_only()
                    on_close=on_close_settings
                    on_set_theme=on_set_theme
                    on_set_default_connection=on_set_default_connection
                    on_set_default_prompt=on_set_default_prompt
                />
            </Show>
        </div>
        </Show>
    }
}
