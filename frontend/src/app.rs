use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{
    ChatMessage, ChatSession, Connection, FileEntry, Project, Role, WorkspaceMode,
};
use web_sys::{AbortController, FileSystemDirectoryHandle};

use crate::api::{BackendApi, HealthState, SseEvent};
use crate::components::{ChatPane, Editor, FileTree, Sidebar, StatusBar, TabBar, TopBar};
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
    search: Option<Vec<FileEntry>>,
    active_session: Option<i64>,
}

/// The directory containing `path` ("" for a top-level path).
fn parent_dir(path: &str) -> String {
    path.rfind('/')
        .map(|i| path[..i].to_string())
        .unwrap_or_default()
}

#[component]
pub fn App() -> impl IntoView {
    let api = BackendApi::from_location();

    // -- global state ------------------------------------------------------
    let health = RwSignal::new(Option::<HealthState>::None);
    let connections = RwSignal::new(Vec::<Connection>::new());
    let projects = RwSignal::new(Vec::<Project>::new());
    let open_tabs = RwSignal::new(Vec::<Project>::new());
    let active_project = RwSignal::new(Option::<i64>::None);
    let sessions = RwSignal::new(Vec::<ChatSession>::new());
    let active_session = RwSignal::new(Option::<i64>::None);
    let messages = RwSignal::new(Vec::<ChatMessage>::new());
    let streaming = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let draft = RwSignal::new(String::new());
    let abort = RwSignal::new(Option::<AbortController>::None);

    // -- active project's workspace state ----------------------------------
    let ws_entries = RwSignal::new(HashMap::<String, Vec<FileEntry>>::new());
    let ws_expanded = RwSignal::new(HashSet::<String>::new());
    let ws_open_file = RwSignal::new(Option::<String>::None);
    let ws_content = RwSignal::new(String::new());
    let ws_dirty = RwSignal::new(false);
    let ws_search = RwSignal::new(Option::<Vec<FileEntry>>::None);
    // Saved workspace state for every project that has (or had) a tab open.
    let saved = RwSignal::new(HashMap::<i64, ProjectWorkspace>::new());
    // Directory handles for local-mode projects, keyed by project id. Loaded
    // from IndexedDB on startup and when a local project is created.
    let local_handles = RwSignal::new(HashMap::<i64, FileSystemDirectoryHandle>::new());

    // -- new-project form state --------------------------------------------
    let show_new_project = RwSignal::new(false);
    let np_name = RwSignal::new(String::new());
    let np_mode = RwSignal::new(WorkspaceMode::Remote);
    let np_path = RwSignal::new("workspace".to_string());

    // -- helpers -----------------------------------------------------------

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

    // Search the project's files.
    let on_search = Callback::new(move |q: String| {
        let Some(pid) = active_project.get() else {
            return;
        };
        spawn_local(async move {
            let Some(ws) = workspace_for.run(pid) else {
                return;
            };
            match ws.search(&q, "").await {
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
        np_path.set("workspace".to_string());
    });

    let on_cancel_new = Callback::new(move |_| {
        show_new_project.set(false);
    });

    // Create a project (open a folder) and open it as a tab.
    let on_create_project = {
        let api = api.clone();
        Callback::new(move |_| {
            let name = np_name.get().trim().to_string();
            if name.is_empty() {
                error.set(Some("Project name is required.".to_string()));
                return;
            }
            let mode = np_mode.get();
            error.set(None);
            let api = api.clone();
            spawn_local(async move {
                // Local mode: pick a directory in the browser, persist its
                // handle, then create the project record.
                if mode == WorkspaceMode::Local {
                    let Ok(handle) = local_fs::pick_directory().await else {
                        return;
                    };
                    let path = handle.name();
                    let Ok(project) = api.create_project(&name, mode, Some(path)).await else {
                        return;
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
                // Remote mode: the path is the folder on the Spin host.
                let path = {
                    let p = np_path.get().trim().to_string();
                    Some(if p.is_empty() {
                        "workspace".to_string()
                    } else {
                        p
                    })
                };
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
            spawn_local(async move {
                match api.create_session(&name, None, None, Some(pid)).await {
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
                        None,
                        Some(&controller.signal()),
                        move |event| {
                            if active_session.get() != Some(session_id) {
                                return;
                            }
                            match event {
                                SseEvent::Message(msg) => {
                                    messages.update(|m| m.push(msg));
                                }
                                SseEvent::Delta(delta) => {
                                    messages.update(|m| {
                                        let extend_last = m
                                            .last()
                                            .is_some_and(|last| last.role == Role::Assistant);
                                        if extend_last {
                                            if let Some(last) = m.last_mut() {
                                                last.content.push_str(&delta);
                                            }
                                        } else {
                                            m.push(ChatMessage {
                                                id: 0,
                                                session_id,
                                                role: Role::Assistant,
                                                content: delta,
                                                created_at: 0,
                                            });
                                        }
                                    });
                                }
                                SseEvent::Done(msg) => {
                                    messages.update(|m| {
                                        if let Some(last) = m.last_mut()
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

    // One-shot initial load: health, connections, projects, sessions, then
    // auto-open the first project as a tab.
    {
        let api = api.clone();
        let sel = select_project;
        Effect::new(move || {
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
                        Ok(m) => messages.set(m),
                        Err(e) => error.set(Some(e)),
                    },
                    None => messages.set(Vec::new()),
                }
            });
        });
    }

    // Derived values for the view.
    let has_session = RwSignal::new(false);
    Effect::new(move || {
        has_session.set(active_session.get().is_some());
    });

    view! {
        <div class="app">
            <TopBar health=health.read_only() />
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
                    on_select_session=on_select_session
                    on_new_session=on_new_session
                    on_rename_session=on_rename_session
                    on_delete_session=on_delete_session
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
                    on_save=on_save
                    error=error.read_only()
                />
                <ChatPane
                    messages=messages.read_only()
                    streaming=streaming.read_only()
                    draft=draft.read_only()
                    set_draft=draft.write_only()
                    error=error.read_only()
                    has_session=has_session.read_only()
                    on_send=on_send
                    on_stop=on_stop
                />
            </div>
            <StatusBar health=health.read_only() />
        </div>
    }
}
