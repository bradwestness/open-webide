use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{
    ChatSession, Connection, ConversationEntry, FileDiff, FileEntry, FileKind, GitCheckoutRequest,
    GitCommitRequest, GitRepoStatus, GitSyncRequest, ModelInfo, Project, ProviderKind, Role,
    SearchHit, SystemPrompt, User, WorkspaceMode,
    tui::{DEFAULT_CONTEXT_LIMIT, EditorContext, SessionTelemetry, SlashCommand},
};
use openwebide_frontend::conversation::{local_message, next_item_nonce};
use web_sys::wasm_bindgen::JsCast;
use web_sys::{AbortController, FileSystemDirectoryHandle};

use crate::api::{BackendApi, HealthState, SseEvent};
use crate::components::{
    AuthGate, ChatPane, ConfirmDialog, ConfirmRequest, ConversationItem, Editor, FileBrowser,
    FileTree, PromptDialog, PromptRequest, Settings, Sidebar, StatusBar, TabBar, TerminalPane,
    ToolStepResult, TopBar, stopped_marker,
};
use crate::idb;
use crate::local_agent;
use crate::local_fs;
use crate::workspace::Workspace;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum ActiveResizer {
    #[default]
    None,
    Sidebar,
    Tree,
    Chat,
}

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
    /// Git repository telemetry status for status bar and file badges.
    git_status: Option<GitRepoStatus>,
    /// Active media/preview URL for images/media.
    media_url: Option<String>,
}

/// The directory containing `path` ("" for a top-level path).
fn parent_dir(path: &str) -> String {
    path.rfind('/')
        .map(|i| path[..i].to_string())
        .unwrap_or_default()
}

fn revoke_object_url(url: Option<String>) {
    if let Some(u) = url {
        let _ = web_sys::Url::revoke_object_url(&u);
    }
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

/// Capture the active editor file, cursor, and any selected line range.
fn capture_active_editor(open_file: Option<String>, content: &str) -> Option<EditorContext> {
    let file_path = open_file?;
    let window = web_sys::window()?;
    let doc = window.document()?;
    let ta_el = doc.query_selector(".editor-textarea").ok()??;
    let ta = ta_el.dyn_into::<web_sys::HtmlTextAreaElement>().ok()?;

    let sel_start = ta.selection_start().ok().flatten().unwrap_or(0) as usize;
    let sel_end = ta.selection_end().ok().flatten().unwrap_or(0) as usize;

    Some(openwebide_frontend::text::editor_context(
        file_path, content, sel_start, sel_end,
    ))
}

fn cancel_run_prompts(items: &mut [ConversationItem], anchor: i64) {
    let prefix = openwebide_agent::step_id_prefix(anchor);
    for item in items {
        if let ConversationItem::ToolStep {
            id,
            awaiting_permission,
            result,
            ..
        } = item
            && id.starts_with(&prefix)
            && *awaiting_permission
            && result.is_none()
        {
            *awaiting_permission = false;
            *result = Some(ToolStepResult {
                ok: false,
                summary: "cancelled".into(),
                diff: None,
            });
        }
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
    let projects_loaded = RwSignal::new(false);
    let sidebar_width = RwSignal::new(240.0f64);
    let tree_width = RwSignal::new(260.0f64);
    let chat_width = RwSignal::new(420.0f64);
    let active_resizer = RwSignal::new(ActiveResizer::None);
    let resizer_start_x = RwSignal::new(0.0f64);
    let resizer_start_w = RwSignal::new(0.0f64);
    let sessions = RwSignal::new(Vec::<ChatSession>::new());
    let active_session = RwSignal::new(Option::<i64>::None);
    let messages = RwSignal::new(Vec::<ConversationItem>::new());
    // A per-run generation for the history-load effect: a slow
    // list_messages for session A must not land after a newer request for
    // the same session A, which the active-session check alone can't catch
    // (a same-id race).
    let history_gen = StoredValue::new(0u64);
    // on_send sets this to a freshly created session's id so the
    // history-load effect's first run for that session skips the fetch,
    // which would wipe the optimistic first message already in `messages`.
    let skip_history_load: StoredValue<Option<i64>> = StoredValue::new(None);
    let streaming = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let draft = RwSignal::new(String::new());
    let abort = RwSignal::new(Option::<AbortController>::None);
    // The session whose run is streaming, so Stop can cancel it server-side.
    let streaming_session = RwSignal::new(Option::<i64>::None);
    let streaming_is_local = RwSignal::new(false);
    // Models reported by the active session's connection, and the model each
    // session has chosen (None = the connection's default).
    let models = RwSignal::new(Vec::<ModelInfo>::new());
    let session_model = RwSignal::new(HashMap::<i64, Option<String>>::new());
    let selected_model = RwSignal::new(Option::<String>::None);

    // -- active project's workspace state ----------------------------------
    let ws_entries = RwSignal::new(HashMap::<String, Vec<FileEntry>>::new());
    let ws_expanded = RwSignal::new(HashSet::<String>::new());
    let ws_open_file = RwSignal::new(Option::<String>::None);
    let ws_content = RwSignal::new(String::new());
    let ws_dirty = RwSignal::new(false);
    let ws_read_only = RwSignal::new(false);
    let needs_grant = RwSignal::new(HashSet::<i64>::new());
    let ws_media_url = RwSignal::new(Option::<String>::None);
    let ws_search = RwSignal::new(Option::<Vec<SearchHit>>::None);
    let ws_pending_edits = RwSignal::new(HashMap::<String, FileDiff>::new());
    // The pending edit for the currently open file (drives the editor's diff
    // view), derived from the open file and the per-project pending edits.
    let pending_diff = RwSignal::new(Option::<FileDiff>::None);
    // Git repository status for the active project.
    let git_status = RwSignal::new(Option::<GitRepoStatus>::None);
    // Saved workspace state for every project that has (or had) a tab open.
    let saved = RwSignal::new(HashMap::<i64, ProjectWorkspace>::new());
    // Directory handles for local-mode projects, keyed by project id. Loaded
    // from IndexedDB on startup and when a local project is created.
    let local_handles = RwSignal::new(HashMap::<i64, FileSystemDirectoryHandle>::new());
    let local_cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let local_permissions =
        std::sync::Arc::new(std::sync::Mutex::new(HashMap::<String, bool>::new()));

    // -- bottom dock terminal & TUI telemetry/context ----------------------
    let show_terminal = RwSignal::new(false);
    let on_toggle_terminal = move || show_terminal.update(|v| *v = !*v);

    let active_editor_context = RwSignal::new(Option::<EditorContext>::None);
    let session_telemetry = RwSignal::new(SessionTelemetry::default());
    let approval_mode =
        RwSignal::new(HashMap::<i64, openwebide_agent::policy::ApprovalMode>::new());
    let current_run_anchor = RwSignal::new(Option::<i64>::None);

    let _ = leptos::prelude::window_event_listener(
        leptos::ev::keydown,
        move |ev: web_sys::KeyboardEvent| {
            // Ctrl+` toggles terminal dock
            if (ev.ctrl_key() || ev.meta_key()) && ev.key() == "`" {
                ev.prevent_default();
                on_toggle_terminal();
                return;
            }

            // Cmd+L / Ctrl+L: capture active editor context and focus prompt input
            if (ev.ctrl_key() || ev.meta_key())
                && (ev.key() == "l" || ev.key() == "L")
                && let Some(ctx) = capture_active_editor(ws_open_file.get(), &ws_content.get())
            {
                ev.prevent_default();
                active_editor_context.set(Some(ctx));
                if let Some(doc) = web_sys::window().and_then(|w| w.document())
                    && let Ok(Some(comp)) = doc.query_selector(".composer-input")
                    && let Ok(el) = comp.dyn_into::<web_sys::HtmlElement>()
                {
                    let _ = el.focus();
                }
                return;
            }

            // Ctrl+K: cycle active focus between composer, editor, sidebar, and terminal
            if (ev.ctrl_key() || ev.meta_key()) && (ev.key() == "k" || ev.key() == "K") {
                ev.prevent_default();
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    let active = doc.active_element();
                    let is_in = |sel: &str| -> bool {
                        if let (Some(a), Ok(Some(target))) =
                            (active.as_ref(), doc.query_selector(sel))
                        {
                            a.is_same_node(Some(&target)) || target.contains(Some(a))
                        } else {
                            false
                        }
                    };

                    if is_in(".composer-input") {
                        if let Ok(Some(el)) = doc.query_selector(".editor-textarea")
                            && let Ok(html_el) = el.dyn_into::<web_sys::HtmlElement>()
                        {
                            let _ = html_el.focus();
                            return;
                        }
                    } else if is_in(".editor-textarea") {
                        if let Ok(Some(el)) = doc.query_selector(".file-tree")
                            && let Ok(html_el) = el.dyn_into::<web_sys::HtmlElement>()
                        {
                            let _ = html_el.focus();
                            return;
                        }
                    } else if (is_in(".sidebar") || is_in(".file-tree"))
                        && show_terminal.get()
                        && let Ok(Some(el)) = doc.query_selector(".terminal-input, .terminal-pane")
                        && let Ok(html_el) = el.dyn_into::<web_sys::HtmlElement>()
                    {
                        let _ = html_el.focus();
                        return;
                    }

                    if let Ok(Some(el)) = doc.query_selector(".composer-input")
                        && let Ok(html_el) = el.dyn_into::<web_sys::HtmlElement>()
                    {
                        let _ = html_el.focus();
                    }
                }
            }
        },
    );

    // Window-level pointer listeners for horizontal panel resizing
    let _ = leptos::prelude::window_event_listener(
        leptos::ev::pointermove,
        move |ev: web_sys::PointerEvent| {
            let active = active_resizer.get();
            if active == ActiveResizer::None {
                return;
            }
            let current_x = ev.client_x();
            let start_x = resizer_start_x.get();
            let start_w = resizer_start_w.get();

            let total_w = web_sys::window()
                .and_then(|w| w.inner_width().ok())
                .and_then(|v| v.as_f64())
                .unwrap_or(1200.0);

            match active {
                ActiveResizer::Sidebar => {
                    let dx = current_x - start_x;
                    let mut new_w = (start_w + dx).clamp(140.0, 480.0);
                    let max_allowed = total_w - tree_width.get() - chat_width.get() - 260.0;
                    if new_w > max_allowed {
                        new_w = max_allowed.max(140.0);
                    }
                    sidebar_width.set(new_w);
                }
                ActiveResizer::Tree => {
                    let dx = current_x - start_x;
                    let mut new_w = (start_w + dx).clamp(160.0, 650.0);
                    let max_allowed = total_w - sidebar_width.get() - chat_width.get() - 260.0;
                    if new_w > max_allowed {
                        new_w = max_allowed.max(160.0);
                    }
                    tree_width.set(new_w);
                }
                ActiveResizer::Chat => {
                    let dx = start_x - current_x;
                    let mut new_w = (start_w + dx).clamp(260.0, 1000.0);
                    let max_allowed = total_w - sidebar_width.get() - tree_width.get() - 260.0;
                    if new_w > max_allowed {
                        new_w = max_allowed.max(260.0);
                    }
                    chat_width.set(new_w);
                }
                ActiveResizer::None => {}
            }
        },
    );

    {
        let api = api.clone();
        let _ = leptos::prelude::window_event_listener(leptos::ev::pointerup, move |_| {
            let active = active_resizer.get();
            if active != ActiveResizer::None {
                active_resizer.set(ActiveResizer::None);
                let s_w = sidebar_width.get();
                let t_w = tree_width.get();
                let c_w = chat_width.get();
                let api = api.clone();
                spawn_local(async move {
                    let _ = api
                        .set_setting("panel_sidebar_width", &s_w.to_string())
                        .await;
                    let _ = api.set_setting("panel_tree_width", &t_w.to_string()).await;
                    let _ = api.set_setting("panel_chat_width", &c_w.to_string()).await;
                });
            }
        });
    }

    // -- system prompts ----------------------------------------------------
    let system_prompts = RwSignal::new(Vec::<SystemPrompt>::new());
    // The create/edit form. `prompt_edit_id` is None when creating, Some(id)
    // when editing an existing prompt.
    let show_prompt_form = RwSignal::new(false);
    let prompt_edit_id = RwSignal::new(Option::<i64>::None);
    let prompt_name = RwSignal::new(String::new());
    let prompt_content = RwSignal::new(String::new());

    // -- connections -------------------------------------------------------
    // The create/edit form. `conn_edit_id` is None when creating, Some(id)
    // when editing an existing connection.
    let show_conn_form = RwSignal::new(false);
    let conn_edit_id = RwSignal::new(Option::<i64>::None);
    let conn_name = RwSignal::new(String::new());
    let conn_kind = RwSignal::new(ProviderKind::Ollama);
    let conn_base_url = RwSignal::new(String::new());
    let conn_model = RwSignal::new(String::new());
    let conn_context_limit = RwSignal::new(String::new());

    // -- settings ----------------------------------------------------------
    let show_settings = RwSignal::new(false);
    let theme = RwSignal::new(read_theme_from_storage());
    // Defaults applied to new sessions (None = the connection's own default).
    let default_connection = RwSignal::new(Option::<i64>::None);
    let default_prompt = RwSignal::new(Option::<i64>::None);

    // -- confirmation dialogs (Phase 10) -----------------------------------
    // A pending confirmation request; the ConfirmDialog renders it and runs
    // its action on confirm. Destructive actions set this instead of using
    // window.confirm.
    let confirm_req = RwSignal::new(Option::<ConfirmRequest>::None);
    let on_close_confirm = Callback::new(move |_| {
        confirm_req.set(None);
    });

    // -- text-input prompts ------------------------------------------------
    // A pending single-field prompt (new file / new folder); the PromptDialog
    // renders it and runs its on_submit with the entered text. Replaces the
    // browser's window.prompt so the input matches the app's theme.
    let prompt_req = RwSignal::new(Option::<PromptRequest>::None);
    let on_close_prompt = Callback::new(move |_| {
        prompt_req.set(None);
    });

    // -- remote file browser (Phase 10) ------------------------------------
    // Opens the host-folder browser so a remote project's folder can be picked
    // by navigating the mount tree. The chosen path is relative to the mount
    // root, which is exactly what NewProject.path wants.
    let show_browser = RwSignal::new(false);
    let on_open_remote = Callback::new(move |_| {
        show_browser.set(true);
    });
    let on_close_browser = Callback::new(move |_| {
        show_browser.set(false);
    });

    // -- git operations (Phase 13) -----------------------------------------
    let refresh_git = Callback::new({
        let api = api.clone();
        move |()| {
            let api = api.clone();
            let pid = active_project.get();
            spawn_local(async move {
                if let Ok(st) = api.git_status(pid).await
                    && active_project.get_untracked() == pid
                {
                    git_status.set(Some(st));
                }
            });
        }
    });

    let on_branch_click = Callback::new({
        let api = api.clone();
        move |()| {
            let api = api.clone();
            let pid = active_project.get();
            prompt_req.set(Some(PromptRequest {
                title: "Switch or Create Git Branch".to_string(),
                value: String::new(),
                placeholder: "Branch name (e.g. feat/my-feature)".to_string(),
                submit_label: "Switch".to_string(),
                on_submit: Callback::new(move |branch: String| {
                    let branch = branch.trim().to_string();
                    if branch.is_empty() {
                        return;
                    }
                    let api = api.clone();
                    spawn_local(async move {
                        let req = GitCheckoutRequest {
                            branch: branch.clone(),
                            create_if_missing: true,
                        };
                        match api.git_checkout(pid, &req).await {
                            Ok(res) => {
                                refresh_git.run(());
                                messages.update(|m| {
                                    m.push(local_message(
                                        active_session.get().unwrap_or(0),
                                        format!(
                                            "Switched to branch `{}` (previous: `{}`).",
                                            res.branch,
                                            res.previous_branch.as_deref().unwrap_or("none")
                                        ),
                                    ));
                                });
                            }
                            Err(e) => {
                                error.set(Some(format!("Git checkout failed: {e}")));
                            }
                        }
                    });
                }),
            }));
        }
    });

    let on_sync_click = Callback::new({
        let api = api.clone();
        move |()| {
            let api = api.clone();
            let pid = active_project.get();
            spawn_local(async move {
                let req = GitSyncRequest {
                    action: "sync".into(),
                    remote: None,
                    branch: None,
                };
                match api.git_sync(pid, &req).await {
                    Ok(res) => {
                        refresh_git.run(());
                        messages.update(|m| {
                            m.push(local_message(
                                active_session.get().unwrap_or(0),
format!(
                                    "Git synchronized with `{}/{}`:\n* Pulled: {} commits\n* Pushed: {} commits",
                                    res.remote, res.branch, res.pulled_commits, res.pushed_commits
                                ),
                            ));
                        });
                    }
                    Err(e) => {
                        error.set(Some(format!("Git sync failed: {e}")));
                    }
                }
            });
        }
    });

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
            let action_api = api.clone();
            let has_unsaved =
                ws_dirty.get_untracked() || saved.get_untracked().values().any(|ws| ws.dirty);
            let mut message =
                "Log out of this account? Open tabs and projects will be closed.".to_string();
            if has_unsaved {
                message.push_str(" Unsaved changes will be lost.");
            }
            confirm_req.set(Some(ConfirmRequest {
                title: "Log out".to_string(),
                message,
                confirm_label: "Log out".to_string(),
                action: Callback::new(move |_| {
                    action_api.set_token(None);
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
                    revoke_object_url(ws_media_url.get());
                    for ws in saved.get().values() {
                        revoke_object_url(ws.media_url.clone());
                    }
                    saved.set(HashMap::new());
                    local_handles.set(HashMap::new());
                    approval_mode.set(HashMap::new());
                    current_run_anchor.set(None);
                    error.set(None);
                }),
            }));
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
                    if e == crate::local_fs::PERMISSION_NEEDED {
                        needs_grant.update(|m| {
                            m.insert(id);
                        });
                    } else if active_project.get() == Some(id) {
                        error.set(Some(e));
                    }
                }
            }
        });
    });

    let on_grant_access = Callback::new(move |_| {
        let Some(pid) = active_project.get() else {
            return;
        };
        let handle = local_handles.with(|m| m.get(&pid).cloned());
        if let Some(handle) = handle {
            let ld = load_dir;
            spawn_local(async move {
                if let Ok(true) = crate::local_fs::request_access(&handle).await {
                    needs_grant.update(|m| {
                        m.remove(&pid);
                    });
                    ld.run((pid, String::new()));
                }
            });
        }
    });

    // Make sure the active project's root directory is loaded.
    let ensure_root = Callback::new(move |id: i64| {
        if ws_entries.get().contains_key("") {
            return;
        }
        load_dir.run((id, String::new()));
    });

    // Open a file: clear the dirty flag and load its contents.
    let on_open_lossy = Callback::new(move |_| {
        let Some(pid) = active_project.get() else {
            return;
        };
        let Some(path) = ws_open_file.get() else {
            return;
        };
        ws_read_only.set(true);
        ws_dirty.set(false);
        spawn_local(async move {
            let Some(ws) = workspace_for.run(pid) else {
                return;
            };
            let content = match ws {
                Workspace::Remote { api, project_id } => {
                    api.read_file_lossy(project_id, &path).await
                }
                Workspace::Local { handle } => crate::local_fs::read_lossy(&handle, &path).await,
            };
            match content {
                Ok(content) => {
                    if active_project.get_untracked() == Some(pid)
                        && ws_open_file.get_untracked().as_deref() == Some(path.as_str())
                    {
                        ws_content.set(content);
                    }
                }
                Err(e) => {
                    if active_project.get_untracked() == Some(pid)
                        && ws_open_file.get_untracked().as_deref() == Some(path.as_str())
                    {
                        error.set(Some(e));
                    }
                }
            }
        });
    });

    // Open a file: clear the dirty flag and load its contents.
    let on_open = Callback::new(move |path: String| {
        let Some(pid) = active_project.get() else {
            return;
        };
        ws_read_only.set(false);
        ws_dirty.set(false);
        ws_open_file.set(Some(path.clone()));
        ws_content.set(String::new());
        revoke_object_url(ws_media_url.get());
        ws_media_url.set(None);
        error.set(None);
        let kind = FileKind::from_path(&path);
        spawn_local(async move {
            let Some(ws) = workspace_for.run(pid) else {
                return;
            };

            if kind == FileKind::Image
                && let Ok(url) = ws.read_blob_url(&path).await
            {
                if active_project.get_untracked() == Some(pid)
                    && ws_open_file.get_untracked().as_deref() == Some(path.as_str())
                {
                    ws_media_url.set(Some(url));
                } else {
                    revoke_object_url(Some(url));
                }
            }

            if kind.is_non_text() {
                // Non-text file (image, binary, archive, etc.): Preview/Placeholder handles it.
                return;
            }

            match ws.read(&path).await {
                Ok(content) => {
                    if active_project.get_untracked() == Some(pid)
                        && ws_open_file.get_untracked().as_deref() == Some(path.as_str())
                    {
                        ws_content.set(content);
                    }
                }
                Err(e) => {
                    if active_project.get_untracked() == Some(pid)
                        && ws_open_file.get_untracked().as_deref() == Some(path.as_str())
                        && !e.contains("not valid UTF-8")
                    {
                        error.set(Some(e));
                    }
                }
            }
        });
    });

    let request_open = Callback::new({
        move |path: String| {
            let current = ws_open_file.get_untracked();
            if ws_dirty.get_untracked() && current.as_deref() != Some(path.as_str()) {
                let current_path = current.unwrap_or_default();
                let path_clone = path.clone();
                confirm_req.set(Some(ConfirmRequest {
                    title: "Discard unsaved changes".to_string(),
                    message: format!(
                        "`{}` has unsaved changes. Discard them and open `{}`?",
                        current_path, path
                    ),
                    confirm_label: "Discard".to_string(),
                    action: Callback::new(move |_| {
                        on_open.run(path_clone.clone());
                    }),
                }));
            } else {
                on_open.run(path);
            }
        }
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
                Ok(()) => {
                    if active_project.get_untracked() == Some(pid)
                        && ws_open_file.get_untracked().as_deref() == Some(path.as_str())
                        && ws_content.get_untracked() == content
                    {
                        ws_dirty.set(false);
                    } else {
                        saved.update(|map| {
                            if let Some(ws) = map.get_mut(&pid)
                                && ws.open_file.as_deref() == Some(path.as_str())
                                && ws.content == content
                            {
                                ws.dirty = false;
                            }
                        });
                    }
                    refresh_git.run(());
                }
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

        if let Some(backup_path) = diff.backup_path {
            spawn_local(async move {
                if let Some(ws) = workspace_for.run(pid) {
                    let _ = ws.delete(&backup_path).await;
                }
            });
        }

        ws_content.set(diff.new);
        ws_dirty.set(false);
        load_dir.run((pid, parent_dir(&path)));
        refresh_git.run(());
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
        let action = openwebide_frontend::pending::reject_action(&diff);
        let action_clone = action.clone();
        let ld = load_dir;
        confirm_req.set(Some(ConfirmRequest {
            title: "Reject edit".to_string(),
            message: match &action {
                openwebide_frontend::pending::RejectAction::Restore(_) => "Reject this edit? The file will be restored to its previous contents.".to_string(),
                openwebide_frontend::pending::RejectAction::RestoreFromBackup(_) => "Reject this edit? The file will be restored from backup.".to_string(),
                openwebide_frontend::pending::RejectAction::Delete => "Reject this edit? The newly created file will be deleted.".to_string(),
                openwebide_frontend::pending::RejectAction::Unavailable => "Cannot reject this edit because the file's previous contents were too large to back up and no copy exists.".to_string(),
            },
            confirm_label: match &action {
                openwebide_frontend::pending::RejectAction::Unavailable => "Ok".to_string(),
                _ => "Reject".to_string(),
            },
            action: Callback::new(move |_| {
                let action = action_clone.clone();
                if matches!(action, openwebide_frontend::pending::RejectAction::Unavailable) {
                    return;
                }
                let path = path.clone();
                ws_pending_edits.update(|m| {
                    m.remove(&path);
                });
                error.set(None);
                spawn_local(async move {
                    let Some(ws) = workspace_for.run(pid) else {
                        return;
                    };
                    let result = match &action {
                        openwebide_frontend::pending::RejectAction::Restore(prev) => {
                            let r = ws.write(&path, prev).await;
                            r.map(|()| prev.clone())
                        }
                        openwebide_frontend::pending::RejectAction::RestoreFromBackup(backup) => {
                            let r = ws.copy(backup, &path).await;
                            if r.is_ok() {
                                let _ = ws.delete(backup).await;
                            }
                            r.map(|()| String::new())
                        }
                        openwebide_frontend::pending::RejectAction::Delete => ws.delete(&path).await.map(|_| String::new()),
                        openwebide_frontend::pending::RejectAction::Unavailable => unreachable!(),
                    };
                    match result {
                        Ok(content) => {
                            if active_project.get() == Some(pid) {
                                if matches!(action, openwebide_frontend::pending::RejectAction::Delete) {
                                    ws_open_file.set(None);
                                } else if matches!(action, openwebide_frontend::pending::RejectAction::RestoreFromBackup(_)) {
                                    ws_open_file.set(None);
                                    ws_open_file.set(Some(path.clone()));
                                } else {
                                    ws_content.set(content);
                                }
                                ws_dirty.set(false);
                                ld.run((pid, parent_dir(&path)));
                                refresh_git.run(());
                            }
                        }
                        Err(e) => {
                            if active_project.get() == Some(pid) {
                                error.set(Some(e));
                            }
                        }
                    }
                });
            }),
        }));
    });

    // Create a new empty file and open it. The path is entered in a themed
    // prompt dialog (not the browser's window.prompt).
    let on_new_file = Callback::new(move |_| {
        if active_project.get().is_none() {
            return;
        }
        prompt_req.set(Some(PromptRequest {
            title: "New file".to_string(),
            value: String::new(),
            placeholder: "Path relative to project (e.g. src/main.rs)".to_string(),
            submit_label: "Create".to_string(),
            on_submit: Callback::new(move |path: String| {
                let Some(pid) = active_project.get() else {
                    return;
                };
                let path = path.trim().to_string();
                if path.is_empty() {
                    return;
                }
                error.set(None);
                let open = request_open;
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
            }),
        }));
    });

    // Create a new directory and refresh its parent. The path is entered in a
    // themed prompt dialog (not the browser's window.prompt).
    let on_new_dir = Callback::new(move |_| {
        if active_project.get().is_none() {
            return;
        }
        prompt_req.set(Some(PromptRequest {
            title: "New folder".to_string(),
            value: String::new(),
            placeholder: "Path relative to project (e.g. src/utils)".to_string(),
            submit_label: "Create".to_string(),
            on_submit: Callback::new(move |path: String| {
                let Some(pid) = active_project.get() else {
                    return;
                };
                let path = path.trim().to_string();
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
            }),
        }));
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
                git_status: git_status.get(),
                media_url: ws_media_url.get(),
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
        git_status.set(ws.git_status);
        ws_media_url.set(ws.media_url);
        error.set(None);
        ensure_root.run(id);
        refresh_git.run(());
    });

    // Close a project tab: the project itself is kept (and stays in the
    // tab bar's "Recent" menu); if the closed tab was active, switch to
    // another open tab.
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
                    git_status.set(None);
                    revoke_object_url(ws_media_url.get());
                    ws_media_url.set(None);
                }
            }
        }
    });

    // Open an existing project (from the tab bar's "Recent" menu) as a tab.
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

    // "Open local": pick a folder in the browser via the File System Access
    // API, persist its handle, then create a Local-mode project named after
    // the picked folder and open it as a tab.
    let on_open_local = {
        let api = api.clone();
        Callback::new(move |_| {
            error.set(None);
            let api = api.clone();
            spawn_local(async move {
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
                let project = match api
                    .create_project(&name, WorkspaceMode::Local, Some(name.clone()))
                    .await
                {
                    Ok(project) => project,
                    Err(e) => {
                        error.set(Some(e));
                        return;
                    }
                };
                // create_project dedups by folder, so re-picking an already
                // saved folder returns the existing project.
                let preexisting = projects.get().iter().any(|p| p.id == project.id);
                if let Err(e) = idb::save_handle(project.id, &handle).await {
                    if !preexisting {
                        let _ = api.delete_project(project.id).await;
                    }
                    error.set(Some(e));
                    return;
                }
                local_handles.update(|m| {
                    m.insert(project.id, handle);
                });
                projects.update(|all| {
                    if !all.iter().any(|p| p.id == project.id) {
                        all.push(project.clone());
                    }
                });
                open_tabs.update(|tabs| {
                    if !tabs.iter().any(|p| p.id == project.id) {
                        tabs.push(project.clone());
                    }
                });
                select_project.run(project.id);
            });
        })
    };

    // "Open remote": a folder was picked in the host-folder browser. Create a
    // Remote-mode project named after the folder (its basename) and open it as
    // a tab. `path` is relative to the mount root (see spin.toml).
    let on_browser_select = {
        let api = api.clone();
        Callback::new(move |path: String| {
            error.set(None);
            let api = api.clone();
            spawn_local(async move {
                let name = folder_name(&path);
                let project = match api
                    .create_project(&name, WorkspaceMode::Remote, Some(path))
                    .await
                {
                    Ok(project) => project,
                    Err(e) => {
                        error.set(Some(e));
                        return;
                    }
                };
                projects.update(|all| {
                    if !all.iter().any(|p| p.id == project.id) {
                        all.push(project.clone());
                    }
                });
                open_tabs.update(|tabs| {
                    if !tabs.iter().any(|p| p.id == project.id) {
                        tabs.push(project.clone());
                    }
                });
                select_project.run(project.id);
            });
        })
    };

    // -- session callbacks -------------------------------------------------

    let on_select_session = Callback::new(move |id: i64| {
        active_session.set(Some(id));
    });

    // "New chat": deselect the active session so the next message starts a
    // fresh one. The session itself is created automatically — and named from
    // the first prompt — when the user sends that message (see on_send).
    let on_new_session = Callback::new(move |_| {
        active_session.set(None);
    });

    let on_rename_session = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let current = sessions
                .with(|list| list.iter().find(|s| s.id == id).map(|s| s.name.clone()))
                .unwrap_or_default();
            let api = api.clone();
            prompt_req.set(Some(PromptRequest {
                title: "Rename session".to_string(),
                value: current,
                placeholder: "Session name".to_string(),
                submit_label: "Rename".to_string(),
                on_submit: Callback::new(move |name: String| {
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
                }),
            }));
        })
    };

    let on_delete_session = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let api = api.clone();
            confirm_req.set(Some(ConfirmRequest {
                title: "Delete session".to_string(),
                message: "Delete this session and its messages?".to_string(),
                confirm_label: "Delete".to_string(),
                action: Callback::new(move |_| {
                    let api = api.clone();
                    spawn_local(async move {
                        if let Err(e) = api.delete_session(id).await {
                            error.set(Some(e));
                            return;
                        }
                        sessions.update(|list| list.retain(|s| s.id != id));
                        approval_mode.update(|m| {
                            m.remove(&id);
                        });
                        if active_session.get() == Some(id) {
                            active_session.set(None);
                        }
                    });
                }),
            }));
        })
    };

    // Delete a project: remove it (and its sessions) from the backend, drop
    // its cached workspace state and local directory handle, then close its
    // tab (switching the active project if it was open).
    let on_delete_project = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let api = api.clone();
            confirm_req.set(Some(ConfirmRequest {
                title: "Delete project".to_string(),
                message: "Delete this project? Its sessions and messages will be removed too."
                    .to_string(),
                confirm_label: "Delete".to_string(),
                action: Callback::new(move |_| {
                    let api = api.clone();
                    spawn_local(async move {
                        if let Err(e) = api.delete_project(id).await {
                            error.set(Some(e));
                            return;
                        }
                        saved.update(|m| {
                            if let Some(ws) = m.remove(&id) {
                                revoke_object_url(ws.media_url);
                            }
                        });
                        local_handles.update(|m| {
                            m.remove(&id);
                        });
                        projects.update(|all| all.retain(|p| p.id != id));
                        sessions.update(|list| list.retain(|s| s.project_id != Some(id)));
                        close_project.run(id);
                    });
                }),
            }));
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
            let api = api.clone();
            confirm_req.set(Some(ConfirmRequest {
                title: "Delete system prompt".to_string(),
                message: "Delete this system prompt?".to_string(),
                confirm_label: "Delete".to_string(),
                action: Callback::new(move |_| {
                    let api = api.clone();
                    spawn_local(async move {
                        if let Err(e) = api.delete_system_prompt(id).await {
                            error.set(Some(e));
                            return;
                        }
                        system_prompts.update(|list| list.retain(|p| p.id != id));
                    });
                }),
            }));
        })
    };

    // -- connection callbacks ----------------------------------------------

    // Show the form to create a new connection.
    let on_new_connection = Callback::new(move |_| {
        show_conn_form.set(true);
        conn_edit_id.set(None);
        conn_name.set(String::new());
        conn_kind.set(ProviderKind::Ollama);
        conn_base_url.set(String::new());
        conn_model.set(String::new());
        conn_context_limit.set(String::new());
    });

    // Show the form to edit an existing connection, loading its values.
    let on_edit_connection = Callback::new(move |id: i64| {
        let Some(c) = connections.get().into_iter().find(|c| c.id == id) else {
            return;
        };
        conn_edit_id.set(Some(id));
        conn_name.set(c.name);
        conn_kind.set(c.kind);
        conn_base_url.set(c.base_url);
        conn_model.set(c.model.unwrap_or_default());
        conn_context_limit.set(c.context_limit.map(|n| n.to_string()).unwrap_or_default());
        show_conn_form.set(true);
    });

    let on_cancel_connection = Callback::new(move |_| {
        show_conn_form.set(false);
    });

    // Create or update the connection, then refresh the list.
    let on_save_connection = {
        let api = api.clone();
        Callback::new(move |_| {
            let name = conn_name.get().trim().to_string();
            if name.is_empty() {
                error.set(Some("Connection name is required.".to_string()));
                return;
            }
            let base_url = conn_base_url.get().trim().to_string();
            if base_url.is_empty() {
                error.set(Some("Base URL is required.".to_string()));
                return;
            }
            let kind = conn_kind.get();
            let model = conn_model.get().trim().to_string();
            let model = if model.is_empty() { None } else { Some(model) };
            let context_limit_input = conn_context_limit.get().trim().to_string();
            let context_limit = if context_limit_input.is_empty() {
                None
            } else {
                match context_limit_input.parse::<usize>() {
                    Ok(n) if n > 0 => Some(n),
                    _ => {
                        error.set(Some(
                            "Context limit must be a positive whole number of tokens.".into(),
                        ));
                        return;
                    }
                }
            };
            let edit_id = conn_edit_id.get();
            error.set(None);
            let api = api.clone();
            spawn_local(async move {
                let result = match edit_id {
                    Some(id) => {
                        // Preserve the existing connection's enabled flag.
                        let enabled = connections
                            .get()
                            .into_iter()
                            .find(|c| c.id == id)
                            .map(|c| c.enabled)
                            .unwrap_or(true);
                        let updated = Connection {
                            id,
                            name: name.clone(),
                            kind,
                            base_url: base_url.clone(),
                            model: model.clone(),
                            enabled,
                            context_limit,
                        };
                        api.update_connection(&updated).await
                    }
                    None => {
                        api.create_connection(
                            &name,
                            kind,
                            &base_url,
                            model.as_deref(),
                            context_limit,
                        )
                        .await
                    }
                };
                match result {
                    Ok(c) => {
                        connections.update(|list| match edit_id {
                            Some(id) => {
                                if let Some(existing) = list.iter_mut().find(|x| x.id == id) {
                                    *existing = c;
                                }
                            }
                            None => list.push(c),
                        });
                        show_conn_form.set(false);
                    }
                    Err(e) => error.set(Some(e)),
                }
            });
        })
    };

    let on_delete_connection = {
        let api = api.clone();
        Callback::new(move |id: i64| {
            let api = api.clone();
            confirm_req.set(Some(ConfirmRequest {
                title: "Delete connection".to_string(),
                message:
                    "Delete this connection? Sessions using it will lose their LLM connection."
                        .to_string(),
                confirm_label: "Delete".to_string(),
                action: Callback::new(move |_| {
                    let api = api.clone();
                    spawn_local(async move {
                        if let Err(e) = api.delete_connection(id).await {
                            error.set(Some(e));
                            return;
                        }
                        connections.update(|list| list.retain(|c| c.id != id));
                    });
                }),
            }));
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

    // -- send / stop / permissions ----------------------------------------

    let on_stop = {
        let api = api.clone();
        let local_cancel = local_cancel_flag.clone();
        Callback::new(move |_| {
            local_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            // Ask the server to stop the run; the browser abort below can't do it.
            if let Some(session_id) = streaming_session.get()
                && !streaming_is_local.get_untracked()
            {
                let api = api.clone();
                spawn_local(async move {
                    let _ = api.cancel_session(session_id).await;
                });
            }
            // The abort tears down the stream before the server's Cancelled
            // event can arrive, so mark the stop here.
            if let Some(anchor) = current_run_anchor.get_untracked() {
                messages.update(|m| {
                    cancel_run_prompts(m, anchor);
                    m.push(stopped_marker());
                });
                current_run_anchor.set(None);
            } else {
                messages.update(|m| m.push(stopped_marker()));
            }
            abort.with(|a| {
                if let Some(c) = a.as_ref() {
                    c.abort();
                }
            });
        })
    };

    let on_permission = {
        let api = api.clone();
        let local_perms = local_permissions.clone();
        Callback::new(move |(tool_call_id, approved): (String, bool)| {
            if let Ok(mut map) = local_perms.lock() {
                map.insert(tool_call_id.clone(), approved);
            }
            // Clear the prompt immediately; the ToolCall (approved) or
            // ToolResult (denied) event that follows confirms it.
            messages.update(|m| {
                if let Some(awaiting) = m.iter_mut().find_map(|item| match item {
                    ConversationItem::ToolStep {
                        id,
                        awaiting_permission,
                        ..
                    } if *id == tool_call_id => Some(awaiting_permission),
                    _ => None,
                }) {
                    *awaiting = false;
                }
            });
            if let Some(session_id) = streaming_session.get() {
                let api = api.clone();
                spawn_local(async move {
                    let _ = api
                        .set_permission(session_id, &tool_call_id, approved)
                        .await;
                });
            }
        })
    };

    let on_permission_always = {
        Callback::new(move |tool_call_id: String| {
            if let Some(session_id) = streaming_session.get_untracked() {
                approval_mode.update(|m| {
                    m.insert(
                        session_id,
                        openwebide_agent::policy::ApprovalMode::AlwaysForSession,
                    );
                });
            } else {
                return;
            }
            on_permission.run((tool_call_id, true));
        })
    };

    let on_send = {
        let api = api.clone();
        let local_cancel = local_cancel_flag.clone();
        let local_perms = local_permissions.clone();
        Callback::new(move |_| {
            let content = draft.with(|d| d.trim().to_string());
            if content.is_empty() || streaming.get() {
                return;
            }
            local_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
            if let Some(pid) = active_project.get_untracked()
                && let Some(p) = projects.get_untracked().into_iter().find(|p| p.id == pid)
            {
                streaming_is_local.set(p.mode == WorkspaceMode::Local);
            }
            if let Ok(mut map) = local_perms.lock() {
                map.clear();
            }
            // Clear the draft and mark streaming synchronously so a second
            // send can't fire while the session is (possibly) created.
            draft.set(String::new());
            streaming.set(true);
            error.set(None);
            let api = api.clone();
            let local_cancel = local_cancel.clone();
            let local_perms = local_perms.clone();
            spawn_local(async move {
                // Resolve the session: use the active one, or — when starting a
                // fresh chat with none selected — create one named from this
                // first prompt.
                let session_id = match active_session.get() {
                    Some(id) => id,
                    None => {
                        let Some(pid) = active_project.get() else {
                            streaming.set(false);
                            return;
                        };
                        let name = derive_session_name(&content);
                        // Use the configured default connection; if none is
                        // set, fall back to the first enabled one so chat works
                        // out of the box.
                        let conn = default_connection.get().or_else(|| {
                            connections
                                .get()
                                .into_iter()
                                .find(|c| c.enabled)
                                .map(|c| c.id)
                        });
                        let prompt = default_prompt.get();
                        match api.create_session(&name, conn, prompt, Some(pid)).await {
                            Ok(s) => {
                                sessions.update(|all| all.push(s.clone()));
                                skip_history_load.set_value(Some(s.id));
                                active_session.set(Some(s.id));
                                s.id
                            }
                            Err(e) => {
                                error.set(Some(e));
                                streaming.set(false);
                                return;
                            }
                        }
                    }
                };
                let model = session_model.get().get(&session_id).cloned().flatten();
                let Ok(controller) = AbortController::new() else {
                    streaming.set(false);
                    return;
                };
                abort.set(Some(controller.clone()));
                streaming_session.set(Some(session_id));

                // Check if this project is local mode and has an active directory handle
                let active_proj = active_project
                    .get()
                    .and_then(|pid| projects.get().into_iter().find(|p| p.id == pid));
                let is_local = active_proj
                    .as_ref()
                    .map(|p| p.mode == WorkspaceMode::Local)
                    .unwrap_or(false);
                let local_handle = active_proj
                    .as_ref()
                    .and_then(|p| local_handles.get().get(&p.id).cloned());

                let run_pid = active_project.get_untracked();
                let on_event = move |event: SseEvent| {
                    if let SseEvent::ToolResult {
                        diff: Some(ref d), ..
                    } = event
                    {
                        if active_project.get_untracked() == run_pid {
                            ws_pending_edits.update(|map| {
                                openwebide_frontend::pending::merge_pending(map, d.clone())
                            });
                        } else if let Some(pid) = run_pid {
                            saved.update(|map| {
                                let entry = map.entry(pid).or_default();
                                openwebide_frontend::pending::merge_pending(
                                    &mut entry.pending_edits,
                                    d.clone(),
                                );
                            });
                        }
                    }

                    if active_session.get() != Some(session_id) {
                        return;
                    }
                    match event {
                        SseEvent::Message(msg) => {
                            if msg.role == Role::User {
                                current_run_anchor.set(Some(msg.id));
                            }
                            messages.update(|m| m.push(ConversationItem::Message(msg)));
                        }
                        SseEvent::Delta(delta) => {
                            messages.update(|m| {
                                let extend_last = m.last().is_some_and(|last| {
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
                                    m.push(local_message(session_id, delta));
                                }
                            });
                        }
                        SseEvent::PermissionRequest { id, name, summary } => {
                            let mode = approval_mode
                                .get_untracked()
                                .get(&session_id)
                                .copied()
                                .unwrap_or_default();
                            if mode.auto_approves(&name) {
                                on_permission.run((id.clone(), true));
                            } else {
                                messages.update(|m| {
                                    m.push(ConversationItem::ToolStep {
                                        key: next_item_nonce(),
                                        id,
                                        name,
                                        summary,
                                        result: None,
                                        awaiting_permission: true,
                                    })
                                });
                            }
                        }
                        SseEvent::ToolCall { id, name, summary } => {
                            session_telemetry.update(|s| s.tool_calls_count += 1);
                            messages.update(|m| {
                                // A gated tool already emitted a
                                // PermissionRequest step; update it
                                // rather than pushing a duplicate.
                                let idx = m.iter().rposition(|item| {
                                    matches!(
                                        item,
                                        ConversationItem::ToolStep { id: tid, .. }
                                            if *tid == id
                                    )
                                });
                                match idx {
                                    Some(i) => {
                                        if let ConversationItem::ToolStep {
                                            summary: s,
                                            awaiting_permission: a,
                                            ..
                                        } = &mut m[i]
                                        {
                                            *s = summary;
                                            *a = false;
                                        }
                                    }
                                    None => m.push(ConversationItem::ToolStep {
                                        key: next_item_nonce(),
                                        id,
                                        name,
                                        summary,
                                        result: None,
                                        awaiting_permission: false,
                                    }),
                                }
                            });
                        }
                        SseEvent::ToolResult {
                            id,
                            name: _,
                            ok,
                            summary,
                            diff,
                        } => {
                            // Record the agent's edit as a pending diff
                            // and open the file so the editor shows it
                            // in diff mode.
                            if let Some(d) = &diff
                                && active_project.get_untracked() == run_pid
                            {
                                if !ws_dirty.get_untracked() {
                                    request_open.run(d.path.clone());
                                } else {
                                    messages.update(|m| {
                                        m.push(local_message(
                                            session_id,
                                            format!(
                                                "Agent edited `{}`; review it with `/diff {}`.",
                                                d.path, d.path
                                            ),
                                        ));
                                    });
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
                                            result: r,
                                            awaiting_permission: a,
                                            ..
                                        } = &mut m[i]
                                        {
                                            *r = Some(ToolStepResult {
                                                ok,
                                                summary: summary.clone(),
                                                diff: diff.clone(),
                                            });
                                            *a = false;
                                        }
                                    }
                                    None => {
                                        m.push(ConversationItem::ToolStep {
                                            key: next_item_nonce(),
                                            id,
                                            name: String::new(),
                                            summary: summary.clone(),
                                            result: Some(ToolStepResult { ok, summary, diff }),
                                            awaiting_permission: false,
                                        });
                                    }
                                }
                            });
                        }
                        SseEvent::Done(msg) => {
                            messages.update(|m| {
                                // Plain chat: replace the streaming
                                // placeholder. Agent mode: the last
                                // item is a tool step (or the user
                                // message), so append the reply.
                                let is_last_assistant = m.last().is_some_and(|last| {
                                    matches!(
                                        last,
                                        ConversationItem::Message(msg)
                                            if msg.role == Role::Assistant
                                    )
                                });
                                if is_last_assistant {
                                    if let Some(ConversationItem::Message(last)) = m.last_mut() {
                                        *last = msg;
                                    }
                                } else {
                                    m.push(ConversationItem::Message(msg));
                                }
                            });
                        }
                        SseEvent::Telemetry(telem) => {
                            session_telemetry.update(|s| s.record_turn(&telem));
                        }
                        SseEvent::Cancelled => {
                            if let Some(anchor) = current_run_anchor.get_untracked() {
                                messages.update(|m| {
                                    cancel_run_prompts(m, anchor);
                                    if !matches!(m.last(), Some(ConversationItem::Stopped { .. })) {
                                        m.push(stopped_marker());
                                    }
                                });
                            } else {
                                messages.update(|m| {
                                    if !matches!(m.last(), Some(ConversationItem::Stopped { .. })) {
                                        m.push(stopped_marker());
                                    }
                                });
                            }
                        }
                        SseEvent::Error(e) => {
                            if let Some(anchor) = current_run_anchor.get_untracked() {
                                messages.update(|m| cancel_run_prompts(m, anchor));
                            }
                            error.set(Some(e));
                        }
                    }
                };

                let ed_ctx = active_editor_context.get();
                active_editor_context.set(None);

                if is_local {
                    if let Some(handle) = local_handle {
                        let session_opt = sessions.get().into_iter().find(|s| s.id == session_id);
                        let connection_id = session_opt
                            .as_ref()
                            .and_then(|s| s.connection_id)
                            .or_else(|| {
                                default_connection.get().or_else(|| {
                                    connections
                                        .get()
                                        .into_iter()
                                        .find(|c| c.enabled)
                                        .map(|c| c.id)
                                })
                            });
                        let Some(conn_id) = connection_id else {
                            error.set(Some(
                                "session has no connection; configure one in settings first".into(),
                            ));
                            streaming.set(false);
                            abort.set(None);
                            streaming_session.set(None);
                            return;
                        };
                        let system_prompt_id = session_opt
                            .as_ref()
                            .and_then(|s| s.system_prompt_id)
                            .or_else(|| default_prompt.get());
                        let system_prompt = system_prompt_id.and_then(|sp_id| {
                            system_prompts
                                .get()
                                .into_iter()
                                .find(|p| p.id == sp_id)
                                .map(|p| p.content)
                        });

                        let vfs = local_fs::BrowserFsaVfs::new(handle);
                        let res = local_agent::run_local_agent(
                            api.clone(),
                            session_id,
                            content,
                            model,
                            ed_ctx,
                            conn_id,
                            system_prompt,
                            vfs,
                            local_cancel,
                            local_perms,
                            on_event,
                        )
                        .await;
                        if let Err(e) = res
                            && !controller.signal().aborted()
                        {
                            error.set(Some(e));
                        }
                    } else {
                        error.set(Some(
                            "local directory handle not available; re-open the folder".into(),
                        ));
                    }
                } else {
                    let result = api
                        .send_message(
                            session_id,
                            &content,
                            model.as_deref(),
                            ed_ctx.as_ref(),
                            Some(&controller.signal()),
                            on_event,
                        )
                        .await;
                    if let Err(e) = result
                        && !controller.signal().aborted()
                    {
                        error.set(Some(e));
                    }
                }

                streaming.set(false);
                abort.set(None);
                streaming_session.set(None);
                current_run_anchor.set(None);
            });
        })
    };

    let on_slash_command = {
        let api = api.clone();
        Callback::new(move |cmd: SlashCommand| match cmd {
            SlashCommand::Help => {
                let help_text = "**Open WebIDE Terminal Execution & Slash Commands**\n\n\
                        **Commands:**\n\
                        * `/help` — Show this cheat sheet\n\
                        * `/model [name]` — Switch model or list available models\n\
                        * `/clear` — Clear the active chat stream\n\
                        * `/diff [path]` — View uncommitted Git diff or pending edits\n\
                        * `/commit <message>` — Stage and commit changes to host Git\n\
                        * `/checkout <branch>` — Switch active Git branch\n\
                        * `/branch <name>` — Create and switch to new Git branch\n\
                        * `/sync` — Synchronize upstream commits (pull & push)\n\
                        * `/test [filter]` — Run tests via execution bridge\n\
                        * `/tokens` or `/context` — Show session token accounting\n\
                        * `/stop` — Abort active execution\n\n\
                        **Keybindings:**\n\
                        * `Cmd+L` / `Ctrl+L` — Capture active editor file & selection into context pill\n\
                        * `Ctrl+` ` — Toggle bottom terminal dock\n\
                        * `Ctrl+K` — Cycle focus between chat, editor, file explorer, and terminal\n\
                        * `Up` / `Down` — Readline prompt history navigation\n\
                        * `Alt+Y` / `Alt+N` / `Alt+A` — Inline permission handshake (approve / deny / always)\n\
                        * `Ctrl+C` / `Esc` — Cancel streaming generation or detach context pill";
                messages.update(|m| {
                    m.push(local_message(active_session.get().unwrap_or(0), help_text));
                });
            }
            SlashCommand::Model(arg) => {
                let all_models = models.get();
                if let Some(target) = arg {
                    if let Some(found) = all_models
                        .iter()
                        .find(|m| m.name.eq_ignore_ascii_case(&target))
                    {
                        let name = found.name.clone();
                        selected_model.set(Some(name.clone()));
                        session_telemetry.update(|t| t.model = name.clone());
                        messages.update(|m| {
                            m.push(local_message(
                                active_session.get().unwrap_or(0),
                                format!("Switched model to `{name}`."),
                            ));
                        });
                    } else {
                        messages.update(|m| {
                            m.push(local_message(
                                active_session.get().unwrap_or(0),
                                format!("Model `{target}` not found in available models."),
                            ));
                        });
                    }
                } else {
                    let names = all_models
                        .iter()
                        .map(|m| format!("* `{}`", m.name))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let cur = selected_model.get().unwrap_or_else(|| "default".into());
                    messages.update(|m| {
                        m.push(local_message(
                                active_session.get().unwrap_or(0),
format!("Current model: `{cur}`\n\nAvailable models:\n{names}\n\nUse `/model <name>` to switch."),
                            ));
                    });
                }
            }
            SlashCommand::Clear => {
                messages.set(Vec::new());
            }
            SlashCommand::Diff(path) => {
                let edits = ws_pending_edits.get();
                if edits.is_empty() {
                    let api = api.clone();
                    let pid = active_project.get();
                    let p_opt = path.clone();
                    spawn_local(async move {
                        match api.git_diff(pid, p_opt.as_deref()).await {
                            Ok(diff) if !diff.trim().is_empty() => {
                                messages.update(|m| {
                                    m.push(local_message(
                                        active_session.get().unwrap_or(0),
                                        format!("**Git Repository Diff:**\n```diff\n{diff}\n```"),
                                    ));
                                });
                            }
                            Ok(_) => {
                                messages.update(|m| {
                                    m.push(local_message(
                                        active_session.get().unwrap_or(0),
                                        "Working tree is clean (no uncommitted diffs).",
                                    ));
                                });
                            }
                            Err(e) => {
                                messages.update(|m| {
                                    m.push(local_message(
                                        active_session.get().unwrap_or(0),
                                        format!("Git diff failed: {e}"),
                                    ));
                                });
                            }
                        }
                    });
                } else if let Some(p) = path {
                    if let Some(diff) = edits.get(&p) {
                        request_open.run(diff.path.clone());
                        messages.update(|m| {
                            m.push(local_message(
                                active_session.get().unwrap_or(0),
                                format!("Opened pending diff for `{}` in editor.", diff.path),
                            ));
                        });
                    } else {
                        messages.update(|m| {
                            m.push(local_message(
                                active_session.get().unwrap_or(0),
                                format!("No pending diff found for `{p}`."),
                            ));
                        });
                    }
                } else {
                    let list = edits
                        .keys()
                        .map(|k| format!("* `{k}`"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    messages.update(|m| {
                            m.push(local_message(
                                active_session.get().unwrap_or(0),
format!("Pending file edits ({count}):\n{list}\n\nUse `/diff <path>` to open in editor.", count = edits.len()),
                            ));
                        });
                }
            }
            SlashCommand::Commit(msg) => {
                let pid = active_project.get();
                let Some(message) = msg.filter(|m| !m.trim().is_empty()) else {
                    messages.update(|m| {
                        m.push(local_message(
                            active_session.get().unwrap_or(0),
                            "Please provide a commit message: `/commit <message>`",
                        ));
                    });
                    return;
                };
                let api = api.clone();
                let refresh_git = refresh_git;
                spawn_local(async move {
                    let req = GitCommitRequest {
                        message: message.clone(),
                        paths: None,
                        include_untracked: false,
                    };
                    match api.git_commit(pid, &req).await {
                        Ok(res) => {
                            refresh_git.run(());
                            messages.update(|m| {
                                m.push(local_message(
                                    active_session.get().unwrap_or(0),
                                    format!(
                                        "Committed `{}`: {}\nSigned: {}",
                                        res.commit_hash, res.summary, res.is_signed
                                    ),
                                ));
                            });
                        }
                        Err(e) => {
                            messages.update(|m| {
                                m.push(local_message(
                                    active_session.get().unwrap_or(0),
                                    format!("Git commit failed: {e}"),
                                ));
                            });
                        }
                    }
                });
            }
            SlashCommand::Checkout(branch_arg) => {
                let pid = active_project.get();
                let Some(branch) = branch_arg.filter(|b| !b.trim().is_empty()) else {
                    messages.update(|m| {
                        m.push(local_message(
                            active_session.get().unwrap_or(0),
                            "Please specify a branch to checkout: `/checkout <branch>`",
                        ));
                    });
                    return;
                };
                let api = api.clone();
                let refresh_git = refresh_git;
                spawn_local(async move {
                    let req = GitCheckoutRequest {
                        branch: branch.clone(),
                        create_if_missing: false,
                    };
                    match api.git_checkout(pid, &req).await {
                        Ok(res) => {
                            refresh_git.run(());
                            messages.update(|m| {
                                m.push(local_message(
                                    active_session.get().unwrap_or(0),
                                    format!(
                                        "Checked out branch `{}` (previous: `{}`).",
                                        res.branch,
                                        res.previous_branch.as_deref().unwrap_or("none")
                                    ),
                                ));
                            });
                        }
                        Err(e) => {
                            messages.update(|m| {
                                m.push(local_message(
                                    active_session.get().unwrap_or(0),
                                    format!("Git checkout failed: {e}"),
                                ));
                            });
                        }
                    }
                });
            }
            SlashCommand::Branch(branch_arg) => {
                let pid = active_project.get();
                let branch_opt = branch_arg.and_then(|b| {
                    let t = b.trim().to_string();
                    if t.is_empty() { None } else { Some(t) }
                });
                let api = api.clone();
                let refresh_git = refresh_git;
                if let Some(branch) = branch_opt {
                    spawn_local(async move {
                        let req = GitCheckoutRequest {
                            branch: branch.clone(),
                            create_if_missing: true,
                        };
                        match api.git_checkout(pid, &req).await {
                            Ok(res) => {
                                refresh_git.run(());
                                messages.update(|m| {
                                    m.push(local_message(
                                        active_session.get().unwrap_or(0),
                                        format!("Created and checked out branch `{}`.", res.branch),
                                    ));
                                });
                            }
                            Err(e) => {
                                messages.update(|m| {
                                    m.push(local_message(
                                        active_session.get().unwrap_or(0),
                                        format!("Git branch failed: {e}"),
                                    ));
                                });
                            }
                        }
                    });
                } else {
                    spawn_local(async move {
                        match api.git_branches(pid).await {
                            Ok(branches) => {
                                let branch_list = if branches.is_empty() {
                                    "No branches found.".to_string()
                                } else {
                                    branches
                                        .into_iter()
                                        .map(|b| {
                                            if b.is_current {
                                                format!("* **{}** (current)", b.name)
                                            } else {
                                                format!("  {}", b.name)
                                            }
                                        })
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                };
                                messages.update(|m| {
                                    m.push(local_message(
                                        active_session.get().unwrap_or(0),
                                        format!("**Repository Branches:**\n\n{}", branch_list),
                                    ));
                                });
                            }
                            Err(e) => {
                                messages.update(|m| {
                                    m.push(local_message(
                                        active_session.get().unwrap_or(0),
                                        format!("Failed to list branches: {e}"),
                                    ));
                                });
                            }
                        }
                    });
                }
            }
            SlashCommand::Sync => {
                on_sync_click.run(());
            }
            SlashCommand::Test(filter) => {
                let arg = filter.unwrap_or_default();
                messages.update(|m| {
                        m.push(local_message(
                            active_session.get().unwrap_or(0),
format!("Dispatched test run: `cargo test {arg}` via execution bridge.\nCheck terminal dock below for full stream."),
                        ));
                    });
                show_terminal.set(true);
            }
            SlashCommand::Tokens => {
                let telem = session_telemetry.get();
                let pct = telem.context_percent();
                let bar = telem.gauge_bar();
                let totals_approx = SessionTelemetry::approx(telem.totals_estimated);
                let context_approx = SessionTelemetry::approx(telem.context_estimated);
                let limit_approx = SessionTelemetry::approx(telem.context_limit_estimated);
                let speed_approx = SessionTelemetry::approx(telem.speed_estimated);
                let speed = telem
                    .current_speed_tps
                    .map(|s| format!("{speed_approx}{s:.1} t/s"))
                    .unwrap_or_else(|| "-- t/s".into());
                let input_tokens = format!("{totals_approx}{}", telem.total_prompt_tokens);
                let output_tokens = format!("{totals_approx}{}", telem.total_completion_tokens);
                let context_tokens = format!("{context_approx}{}", telem.context_tokens);
                let context_limit = format!("{limit_approx}{}", telem.context_limit);
                let text = format!(
                    "```text\n\
                         ┌─ Session Token Accounting ────────────────────────────────────┐\n\
                         │ Model:             {:<42} │\n\
                         │ Input Tokens:      {:<42} │\n\
                         │ Output Tokens:     {:<42} │\n\
                         │ Context:           {} / {} ({:.1}%) {} │\n\
                         │ Current Speed:     {:<42} │\n\
                         │ Tool Calls:        {:<42} │\n\
                         └───────────────────────────────────────────────────────────────┘\n```",
                    telem.model,
                    input_tokens,
                    output_tokens,
                    context_tokens,
                    context_limit,
                    pct,
                    bar,
                    speed,
                    telem.tool_calls_count,
                );
                messages.update(|m| {
                    m.push(local_message(active_session.get().unwrap_or(0), text));
                });
            }
            SlashCommand::Stop => {
                on_stop.run(());
            }
        })
    };

    // Resolve the effective model (the override, else the active session's
    // connection default, else "default"), publish it to the statusline, and
    // fetch the model's context window for the gauge.
    let ctx_request_gen = StoredValue::new(0u64);
    {
        let api = api.clone();
        Effect::new(move || {
            // Bump and capture the generation first, before any early
            // return below: a run that returns early (no connection) must
            // still invalidate an earlier in-flight request, or that
            // request can land after this run's default and overwrite it.
            let this_gen = {
                ctx_request_gen.update_value(|g| *g += 1);
                ctx_request_gen.get_value()
            };
            let sid = active_session.get();
            let conn_id = sid.and_then(|id| {
                sessions
                    .get()
                    .into_iter()
                    .find(|s| s.id == id)
                    .and_then(|s| s.connection_id)
            });
            let conn_model = conn_id.and_then(|cid| {
                connections
                    .get()
                    .into_iter()
                    .find(|c| c.id == cid)
                    .and_then(|c| c.model)
            });
            // `None` here means no real model resolved; only the display
            // string falls back to "default" (sending it as the model
            // would ask the provider to resolve a model literally named
            // "default").
            let effective_model = selected_model.get().or(conn_model);
            let display_model = effective_model
                .clone()
                .unwrap_or_else(|| "default".to_string());
            session_telemetry.update(|t| t.model = display_model);

            let Some(cid) = conn_id else {
                session_telemetry.update(|t| {
                    t.context_limit = DEFAULT_CONTEXT_LIMIT;
                    t.context_limit_estimated = true;
                });
                return;
            };
            let api = api.clone();
            spawn_local(async move {
                let result = api.model_context(cid, effective_model.as_deref()).await;
                if ctx_request_gen.get_value() != this_gen {
                    return;
                }
                let resolved = result.ok().flatten();
                let limit = resolved.unwrap_or(DEFAULT_CONTEXT_LIMIT);
                let estimated = resolved.is_none();
                session_telemetry.update(|t| {
                    t.context_limit = limit;
                    t.context_limit_estimated = estimated;
                });
            });
        });
    }

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
                let mut db_open_ids: Vec<i64> = Vec::new();
                let mut db_active_project: Option<i64> = None;
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
                    if let Some(json) = s.get("open_tabs")
                        && let Ok(ids) = serde_json::from_str::<Vec<i64>>(json)
                    {
                        db_open_ids = ids;
                    }
                    if let Some(act) = s.get("active_project").and_then(|v| v.parse::<i64>().ok()) {
                        db_active_project = Some(act);
                    }
                    if let Some(v) = s
                        .get("panel_sidebar_width")
                        .and_then(|v| v.parse::<f64>().ok())
                    {
                        sidebar_width.set(v.clamp(140.0, 480.0));
                    }
                    if let Some(v) = s
                        .get("panel_tree_width")
                        .and_then(|v| v.parse::<f64>().ok())
                    {
                        tree_width.set(v.clamp(160.0, 650.0));
                    }
                    if let Some(v) = s
                        .get("panel_chat_width")
                        .and_then(|v| v.parse::<f64>().ok())
                    {
                        chat_width.set(v.clamp(260.0, 1000.0));
                    }
                }
                let all_projects = projects.get();
                let mut restored_tabs = Vec::new();
                for tid in &db_open_ids {
                    if let Some(proj) = all_projects.iter().find(|p| p.id == *tid)
                        && !restored_tabs.iter().any(|r: &Project| r.id == proj.id)
                    {
                        restored_tabs.push(proj.clone());
                    }
                }
                if restored_tabs.is_empty()
                    && let Some(first) = all_projects.into_iter().next()
                {
                    restored_tabs.push(first);
                }
                open_tabs.set(restored_tabs.clone());

                let target_active = db_active_project
                    .filter(|act_id| restored_tabs.iter().any(|t| t.id == *act_id))
                    .or_else(|| restored_tabs.first().map(|t| t.id));

                if let Some(act_id) = target_active {
                    sel.run(act_id);
                }

                projects_loaded.set(true);
            });
        });
    }

    // Keep open tabs persisted in the database so the user can pick up on any machine.
    {
        let api = api.clone();
        Effect::new(move || {
            if !projects_loaded.get() {
                return;
            }
            let tabs = open_tabs.get();
            let ids: Vec<i64> = tabs.iter().map(|p| p.id).collect();
            if let Ok(json) = serde_json::to_string(&ids) {
                let api = api.clone();
                spawn_local(async move {
                    let _ = api.set_setting("open_tabs", &json).await;
                });
            }
        });
    }

    // Keep the active project persisted in the database so the user can pick up on any machine.
    {
        let api = api.clone();
        Effect::new(move || {
            if !projects_loaded.get() {
                return;
            }
            let act = active_project.get();
            let val = act.map(|id| id.to_string()).unwrap_or_default();
            let api = api.clone();
            spawn_local(async move {
                let _ = api.set_setting("active_project", &val).await;
            });
        });
    }

    // Load the message history whenever the active session changes.
    {
        let api = api.clone();
        Effect::new(move || {
            let id = active_session.get();
            // A brand-new session (just created by on_send) has no server
            // history yet; skip the fetch so its optimistic first message
            // is not wiped the instant the session becomes active.
            if let Some(sid) = id
                && skip_history_load.get_value() == Some(sid)
            {
                skip_history_load.set_value(None);
                return;
            }
            // Bump and capture the generation: a slow list_messages for
            // session A must not land after a newer request for the same
            // session A, which the active-session check below can't catch.
            let this_gen = {
                history_gen.update_value(|g| *g += 1);
                history_gen.get_value()
            };
            let api = api.clone();
            spawn_local(async move {
                match id {
                    Some(id) => {
                        let result = api.list_messages(id).await;
                        // Drop a stale response, for either arm: the active
                        // session may have changed again while this request
                        // was in flight (e.g. a quick A -> B -> A switch, or
                        // a stale error landing after B is already showing).
                        // The generation additionally catches a same-id race
                        // the session check can't: a newer request for the
                        // same session supersedes this one.
                        if history_gen.get_value() != this_gen
                            || active_session.get_untracked() != Some(id)
                        {
                            return;
                        }
                        match result {
                            Ok(entries) => {
                                session_telemetry.update(|t| t.restore_from_conversation(&entries));
                                messages.set(
                                    entries
                                        .into_iter()
                                        .map(|e| match e {
                                            ConversationEntry::Message(m) => {
                                                ConversationItem::Message(m)
                                            }
                                            ConversationEntry::ToolStep(ts) => {
                                                ConversationItem::ToolStep {
                                                    key: next_item_nonce(),
                                                    id: ts.tool_call_id,
                                                    name: ts.name,
                                                    summary: ts.summary,
                                                    result: ts.ok.map(|ok| ToolStepResult {
                                                        ok,
                                                        summary: ts
                                                            .result_summary
                                                            .clone()
                                                            .unwrap_or_default(),
                                                        diff: ts.diff.clone(),
                                                    }),
                                                    awaiting_permission: false,
                                                }
                                            }
                                        })
                                        .collect(),
                                );
                            }
                            Err(e) => {
                                // Reset telemetry too, so the previous
                                // session's context/totals/speed don't
                                // linger on screen.
                                session_telemetry.update(|t| t.restore_from_conversation(&[]));
                                error.set(Some(e));
                            }
                        }
                    }
                    None => {
                        session_telemetry.update(|t| t.restore_from_conversation(&[]));
                        messages.set(Vec::new());
                    }
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

    let git_head_content =
        RwSignal::new(Option::<(Option<i64>, String, Result<String, String>)>::None);

    let git_head_diff = Signal::derive(move || {
        let open = ws_open_file.get()?;
        let pid = active_project.get();
        let (head_pid, head_path, head_res) = git_head_content.get()?;
        if head_pid != pid || head_path != open {
            return None;
        }
        if head_res.is_err() {
            return None;
        }
        let old = head_res.ok();
        let new = ws_content.get();
        if old.is_none() && new.is_empty() {
            return None;
        }
        Some(FileDiff {
            path: open,
            old,
            new,
            old_unavailable: false,
            backup_path: None,
        })
    });

    let can_revert = Signal::derive(move || {
        if let Some(open) = ws_open_file.get() {
            let pid = active_project.get();
            if let Some((head_pid, head_path, head_res)) = git_head_content.get() {
                return head_pid == pid && head_path == open && head_res.is_ok();
            }
        }
        false
    });

    // Reset git_head_content when open file changes
    Effect::new(move || {
        let _ = ws_open_file.get();
        git_head_content.set(None);
    });

    let on_load_git_diff = {
        let api = api_ref.get();
        Callback::new(move |_| {
            let Some(file_path) = ws_open_file.get() else {
                return;
            };
            let pid = active_project.get();
            let api = api.clone();
            spawn_local(async move {
                match api.git_file_head(pid, &file_path).await {
                    Ok(content) => {
                        if active_project.get_untracked() == pid
                            && ws_open_file.get_untracked().as_deref() == Some(file_path.as_str())
                        {
                            git_head_content.set(Some((pid, file_path.clone(), Ok(content))));
                        }
                    }
                    Err(e) => {
                        if active_project.get_untracked() == pid
                            && ws_open_file.get_untracked().as_deref() == Some(file_path.as_str())
                        {
                            git_head_content.set(Some((pid, file_path.clone(), Err(e.clone()))));
                            error.set(Some(format!("Could not load HEAD: {e}")));
                        }
                    }
                }
            });
        })
    };

    let on_discard_git_diff = {
        Callback::new(move |_| {
            let Some((Some(head_pid), head_path, Ok(head))) = git_head_content.get() else {
                return;
            };
            confirm_req.set(Some(ConfirmRequest {
                title: "Revert to HEAD".to_string(),
                message: format!(
                    "Discard all changes to `{}` and restore the committed version?",
                    head_path
                ),
                confirm_label: "Revert".to_string(),
                action: Callback::new(move |_| {
                    let head = head.clone();
                    let head_path = head_path.clone();
                    spawn_local(async move {
                        let Some(ws) = workspace_for.run(head_pid) else {
                            return;
                        };
                        match ws.write(&head_path, &head).await {
                            Ok(()) => {
                                if active_project.get_untracked() == Some(head_pid)
                                    && ws_open_file.get_untracked().as_deref()
                                        == Some(head_path.as_str())
                                {
                                    ws_content.set(head);
                                    ws_dirty.set(false);
                                    refresh_git.run(());
                                }
                            }
                            Err(e) => {
                                error.set(Some(e));
                            }
                        }
                    });
                }),
            }));
        })
    };

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
                    projects=projects.read_only()
                    active_project=active_project.read_only()
                    on_select=select_project
                    on_close=close_project
                    on_open_local=on_open_local
                    on_open_remote=on_open_remote
                    on_open_project=on_open_project
                    on_delete_project=on_delete_project
                />
            <div class=move || if active_resizer.get() != ActiveResizer::None { "app-body is-resizing" } else { "app-body" }>
                <Sidebar
                    width=Signal::from(sidebar_width.read_only())
                    connections=connections.read_only()
                    show_conn_form=show_conn_form.read_only()
                    conn_edit_id=conn_edit_id.read_only()
                    conn_name=conn_name.read_only()
                    set_conn_name=conn_name.write_only()
                    conn_kind=conn_kind.read_only()
                    set_conn_kind=conn_kind.write_only()
                    conn_base_url=conn_base_url.read_only()
                    set_conn_base_url=conn_base_url.write_only()
                    conn_model=conn_model.read_only()
                    set_conn_model=conn_model.write_only()
                    conn_context_limit=conn_context_limit.read_only()
                    set_conn_context_limit=conn_context_limit.write_only()
                    on_new_connection=on_new_connection
                    on_edit_connection=on_edit_connection
                    on_save_connection=on_save_connection
                    on_cancel_connection=on_cancel_connection
                    on_delete_connection=on_delete_connection
                    sessions=sessions.read_only()
                    active_project=active_project.read_only()
                    active_session=active_session.read_only()
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
                <div
                    class=move || if active_resizer.get() == ActiveResizer::Sidebar { "panel-resizer is-active" } else { "panel-resizer" }
                    title="Drag to resize sidebar, double-click to reset"
                    on:pointerdown=move |ev: web_sys::PointerEvent| {
                        ev.prevent_default();
                        resizer_start_x.set(ev.client_x());
                        resizer_start_w.set(sidebar_width.get());
                        active_resizer.set(ActiveResizer::Sidebar);
                    }
                    on:dblclick={
                        let api = api_ref.get();
                        move |_| {
                            sidebar_width.set(240.0);
                            let api = api.clone();
                            spawn_local(async move {
                                let _ = api.set_setting("panel_sidebar_width", "240").await;
                            });
                        }
                    }
                />
                <FileTree
                    width=Signal::from(tree_width.read_only())
                    entries=ws_entries.read_only()
                    expanded=ws_expanded.read_only()
                    open_file=ws_open_file.read_only()
                    search_results=ws_search.read_only()
                    needs_grant=Signal::derive(move || active_project.get().map(|id| needs_grant.with(|m| m.contains(&id))).unwrap_or(false))
                    on_grant_access=on_grant_access
                    on_toggle=on_toggle
                    on_open=request_open
                    on_new_file=on_new_file
                    on_new_dir=on_new_dir
                    on_search=on_search
                    on_clear_search=on_clear_search
                    git_status=git_status.read_only().into()
                />
                <div
                    class=move || if active_resizer.get() == ActiveResizer::Tree { "panel-resizer is-active" } else { "panel-resizer" }
                    title="Drag to resize file tree / diff viewer, double-click to reset"
                    on:pointerdown=move |ev: web_sys::PointerEvent| {
                        ev.prevent_default();
                        resizer_start_x.set(ev.client_x());
                        resizer_start_w.set(tree_width.get());
                        active_resizer.set(ActiveResizer::Tree);
                    }
                    on:dblclick={
                        let api = api_ref.get();
                        move |_| {
                            tree_width.set(260.0);
                            let api = api.clone();
                            spawn_local(async move {
                                let _ = api.set_setting("panel_tree_width", "260").await;
                            });
                        }
                    }
                />
                <div class="center-pane">
                    <Editor
                        open_file=ws_open_file.read_only()
                        content=ws_content.read_only()
                        set_content=ws_content.write_only()
                        dirty=ws_dirty.read_only()
                        set_dirty=ws_dirty.write_only()
                        pending_diff=pending_diff.read_only()
                        read_only=ws_read_only.read_only().into()
                        on_open_lossy=on_open_lossy
                        media_url=ws_media_url.read_only().into()
                        git_head_diff=git_head_diff
                        on_load_git_diff=on_load_git_diff
                        on_discard_git_diff=on_discard_git_diff
                        can_revert=can_revert
                        on_save=on_save
                        on_accept=on_accept
                        on_reject=on_reject
                    />
                    <Show when=move || show_terminal.get() fallback=|| ()>
                        <TerminalPane on_close=move || show_terminal.set(false) />
                    </Show>
                </div>
                <div
                    class=move || if active_resizer.get() == ActiveResizer::Chat { "panel-resizer is-active" } else { "panel-resizer" }
                    title="Drag to resize diff viewer / chat pane, double-click to reset"
                    on:pointerdown=move |ev: web_sys::PointerEvent| {
                        ev.prevent_default();
                        resizer_start_x.set(ev.client_x());
                        resizer_start_w.set(chat_width.get());
                        active_resizer.set(ActiveResizer::Chat);
                    }
                    on:dblclick={
                        let api = api_ref.get();
                        move |_| {
                            chat_width.set(420.0);
                            let api = api.clone();
                            spawn_local(async move {
                                let _ = api.set_setting("panel_chat_width", "420").await;
                            });
                        }
                    }
                />
                <ChatPane
                    width=Signal::from(chat_width.read_only())
                    messages=messages.read_only()
                    streaming=streaming.read_only()
                    draft=draft.read_only()
                    set_draft=draft.write_only()
                    has_session=has_session.read_only()
                    local_mode=local_mode.read_only()
                    models=models.read_only()
                    selected_model=selected_model.read_only()
                    on_select_model=on_select_model
                    on_send=on_send
                    on_stop=on_stop
                    on_permission=on_permission
                    on_permission_always=on_permission_always
                    active_context=active_editor_context.read_only()
                    set_active_context=active_editor_context.write_only()
                    current_run_anchor=current_run_anchor.read_only().into()
                    session_telemetry=session_telemetry.read_only()
                    on_slash_command=on_slash_command
                />
            </div>
            <StatusBar
                health=health.read_only()
                show_terminal=show_terminal.read_only()
                on_toggle_terminal=on_toggle_terminal
                git_status=git_status.read_only().into()
                on_branch_click=on_branch_click
                on_sync_click=on_sync_click
                approval_mode=Signal::derive(move || active_session.get().and_then(|id| approval_mode.get().get(&id).copied()).unwrap_or_default())
            />
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
            <Show when=move || error.get().is_some() fallback=|| ()>
                <div class="toast" role="alert">
                    <span class="toast-message">{move || error.get().unwrap_or_default()}</span>
                    <button class="icon-btn toast-close" title="Dismiss" on:click=move |_| error.set(None)>
                        "✕"
                    </button>
                </div>
            </Show>
            <ConfirmDialog req=confirm_req.read_only() on_close=on_close_confirm />
            <PromptDialog req=prompt_req.read_only() on_close=on_close_prompt />
            <Show when=move || show_browser.get() fallback=|| ()>
                <FileBrowser
                    api=api_ref.get()
                    on_close=on_close_browser
                    on_select=on_browser_select
                />
            </Show>
        </div>
        </Show>
    }
}

/// Derive a project name from a host-folder path: the folder's basename. An
/// empty path (the mount root) falls back to the root folder's name.
fn folder_name(path: &str) -> String {
    path.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("source")
        .to_string()
}

/// Derive a session name from the first prompt: the first non-empty line,
/// trimmed to a short length. Used to auto-name a session created when the
/// user sends their first message in a fresh chat.
fn derive_session_name(prompt: &str) -> String {
    let line = prompt
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut name: String = line.chars().take(40).collect();
    if line.chars().count() > 40 {
        name.push('…');
    }
    name
}
