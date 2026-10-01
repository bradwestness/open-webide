use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{WorkspaceMode, tui::EditorContext};
use openwebide_frontend::state::{
    auth::AuthState,
    chat::ChatState,
    layout::{ActiveResizer, LayoutState},
    projects::ProjectsState,
    settings::{SettingsState, Theme},
    workspace::WorkspaceState,
};
use wasm_bindgen::JsCast;

use crate::api::{BackendApi, HealthState};

pub struct ProjectEffectContext {
    pub api: BackendApi,
    pub health: RwSignal<Option<HealthState>>,
    pub auth: AuthState,
    pub settings: SettingsState,
    pub projects: ProjectsState,
    pub chat: ChatState,
    pub layout: LayoutState,
    pub select_project: Callback<i64>,
}

pub fn install_keyboard_shortcuts(workspace: WorkspaceState, chat: ChatState) {
    let open_file = workspace.open_file;
    let content = workspace.content;
    let active_editor_context = chat.active_editor_context;
    let show_terminal = chat.show_terminal;
    let on_toggle_terminal = move || show_terminal.update(|value| *value = !*value);

    let _ = leptos::prelude::window_event_listener(
        leptos::ev::keydown,
        move |event: web_sys::KeyboardEvent| {
            if (event.ctrl_key() || event.meta_key()) && event.key() == "`" {
                event.prevent_default();
                on_toggle_terminal();
                return;
            }
            if (event.ctrl_key() || event.meta_key())
                && (event.key() == "l" || event.key() == "L")
                && let Some(context) = capture_active_editor(open_file.get(), &content.get())
            {
                event.prevent_default();
                active_editor_context.set(Some(context));
                if let Some(document) = web_sys::window().and_then(|window| window.document())
                    && let Ok(Some(composer)) = document.query_selector(".composer-input")
                    && let Ok(element) = composer.dyn_into::<web_sys::HtmlElement>()
                {
                    let _ = element.focus();
                }
                return;
            }
            if (event.ctrl_key() || event.meta_key()) && (event.key() == "k" || event.key() == "K")
            {
                event.prevent_default();
                if let Some(document) = web_sys::window().and_then(|window| window.document()) {
                    let active = document.active_element();
                    let is_in = |selector: &str| -> bool {
                        if let (Some(active), Ok(Some(target))) =
                            (active.as_ref(), document.query_selector(selector))
                        {
                            active.is_same_node(Some(&target)) || target.contains(Some(active))
                        } else {
                            false
                        }
                    };

                    if is_in(".composer-input") {
                        if let Ok(Some(element)) = document.query_selector(".editor-textarea")
                            && let Ok(element) = element.dyn_into::<web_sys::HtmlElement>()
                        {
                            let _ = element.focus();
                            return;
                        }
                    } else if is_in(".editor-textarea") {
                        if let Ok(Some(element)) = document.query_selector(".file-tree")
                            && let Ok(element) = element.dyn_into::<web_sys::HtmlElement>()
                        {
                            let _ = element.focus();
                            return;
                        }
                    } else if (is_in(".sidebar") || is_in(".file-tree"))
                        && show_terminal.get()
                        && let Ok(Some(element)) =
                            document.query_selector(".terminal-input, .terminal-pane")
                        && let Ok(element) = element.dyn_into::<web_sys::HtmlElement>()
                    {
                        let _ = element.focus();
                        return;
                    }

                    if let Ok(Some(element)) = document.query_selector(".composer-input")
                        && let Ok(element) = element.dyn_into::<web_sys::HtmlElement>()
                    {
                        let _ = element.focus();
                    }
                }
            }
        },
    );
}

fn capture_active_editor(open_file: Option<String>, content: &str) -> Option<EditorContext> {
    let file_path = open_file?;
    let window = web_sys::window()?;
    let document = window.document()?;
    let textarea = document.query_selector(".editor-textarea").ok()??;
    let textarea = textarea.dyn_into::<web_sys::HtmlTextAreaElement>().ok()?;
    let selection_start = textarea.selection_start().ok().flatten().unwrap_or(0) as usize;
    let selection_end = textarea.selection_end().ok().flatten().unwrap_or(0) as usize;
    Some(openwebide_frontend::text::editor_context(
        file_path,
        content,
        selection_start,
        selection_end,
    ))
}

pub fn install_project_effects(context: ProjectEffectContext) {
    let ProjectEffectContext {
        api,
        health,
        auth,
        settings,
        projects,
        chat,
        layout,
        select_project,
    } = context;
    Effect::new(move |_| {
        if auth.user.get().is_none() {
            return;
        }
        spawn_local(async move {
            let backend_ok = match api.health().await {
                Ok(health_state) => {
                    health.set(Some(HealthState::Online {
                        version: health_state.version,
                    }));
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

            let (
                settings_result,
                projects_result,
                connections_result,
                sessions_result,
                prompts_result,
            ) = futures::join!(
                api.get_settings(),
                api.list_projects(),
                api.list_connections(),
                api.list_sessions(),
                api.list_system_prompts(),
            );
            if let Ok(connections) = connections_result {
                settings.connections.set(connections);
            }
            if let Ok(project_list) = projects_result {
                projects.projects.set(project_list);
            }
            for project in projects
                .projects
                .get()
                .into_iter()
                .filter(|project| project.mode == WorkspaceMode::Local)
            {
                if let Ok(Some(handle)) = crate::idb::load_handle(project.id).await {
                    projects.local_handles.update(|handles| {
                        handles.insert(project.id, handle);
                    });
                }
            }
            if let Ok(sessions) = sessions_result {
                chat.sessions.set(sessions);
            }
            if let Ok(prompts) = prompts_result {
                settings.system_prompts.set(prompts);
            }

            let mut stored_tab_ids = Vec::<i64>::new();
            let mut stored_active_project = None;
            if let Ok(values) = settings_result {
                if let Some(theme) = values.get("theme") {
                    settings.theme.set(Theme::parse(theme));
                }
                if let Some(url) = values.get("bridge_url") {
                    settings.bridge_url.set(url.to_string());
                }
                if let Some(id) = values
                    .get("default_connection")
                    .and_then(|value| value.parse::<i64>().ok())
                {
                    settings.default_connection.set(Some(id));
                }
                if let Some(id) = values
                    .get("default_prompt")
                    .and_then(|value| value.parse::<i64>().ok())
                {
                    settings.default_prompt.set(Some(id));
                }
                if let Some(json) = values.get("open_tabs")
                    && let Ok(ids) = serde_json::from_str::<Vec<i64>>(json)
                {
                    stored_tab_ids = ids;
                }
                if let Some(id) = values
                    .get("active_project")
                    .and_then(|value| value.parse::<i64>().ok())
                {
                    stored_active_project = Some(id);
                }
                let total_width = web_sys::window()
                    .and_then(|window| window.inner_width().ok())
                    .and_then(|value| value.as_f64())
                    .unwrap_or(1200.0);
                if let Some(width) = values
                    .get("panel_sidebar_width")
                    .and_then(|value| value.parse().ok())
                {
                    layout.sidebar_width.set(LayoutState::clamp(
                        ActiveResizer::Sidebar,
                        width,
                        layout.sidebar_width.get_untracked(),
                        layout.tree_width.get_untracked(),
                        layout.chat_width.get_untracked(),
                        total_width,
                    ));
                }
                if let Some(width) = values
                    .get("panel_tree_width")
                    .and_then(|value| value.parse().ok())
                {
                    layout.tree_width.set(LayoutState::clamp(
                        ActiveResizer::Tree,
                        width,
                        layout.sidebar_width.get_untracked(),
                        layout.tree_width.get_untracked(),
                        layout.chat_width.get_untracked(),
                        total_width,
                    ));
                }
                if let Some(width) = values
                    .get("panel_chat_width")
                    .and_then(|value| value.parse().ok())
                {
                    layout.chat_width.set(LayoutState::clamp(
                        ActiveResizer::Chat,
                        width,
                        layout.sidebar_width.get_untracked(),
                        layout.tree_width.get_untracked(),
                        layout.chat_width.get_untracked(),
                        total_width,
                    ));
                }
            }

            let all_projects = projects.projects.get();
            let mut restored_tab_ids = Vec::new();
            for id in &stored_tab_ids {
                if all_projects.iter().any(|project| project.id == *id)
                    && !restored_tab_ids.contains(id)
                {
                    restored_tab_ids.push(*id);
                }
            }
            if restored_tab_ids.is_empty()
                && let Some(first) = all_projects.first()
            {
                restored_tab_ids.push(first.id);
            }
            projects.open_tab_ids.set(restored_tab_ids.clone());
            let active_project = stored_active_project
                .filter(|id| restored_tab_ids.contains(id))
                .or_else(|| restored_tab_ids.first().copied());
            if let Some(project_id) = active_project {
                select_project.run(project_id);
            }
            projects.projects_loaded.set(true);
        });
    });

    Effect::new(move |_| {
        if !projects.projects_loaded.get() {
            return;
        }
        let ids = projects.open_tab_ids.get();
        if let Ok(json) = serde_json::to_string(&ids) {
            spawn_local(async move {
                let _ = api.set_setting("open_tabs", &json).await;
            });
        }
    });

    Effect::new(move |_| {
        if !projects.projects_loaded.get() {
            return;
        }
        let active_project = projects.active_project.get();
        let value = active_project.map(|id| id.to_string()).unwrap_or_default();
        spawn_local(async move {
            let _ = api.set_setting("active_project", &value).await;
        });
    });
}
