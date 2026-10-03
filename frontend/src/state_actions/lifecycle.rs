use crate::state::{
    auth::AuthState,
    chat::ChatState,
    layout::LayoutState,
    projects::ProjectsState,
    settings::{SettingsState, Theme},
    workspace::WorkspaceState,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use openwebide_core::{WorkspaceMode, tui::EditorContext};
use wasm_bindgen::JsCast;

use crate::{api::HealthState, backend::Api};

pub struct ProjectEffectContext {
    pub api: Api,
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
    let select_approval_mode = super::approvals::mode_selector(chat);
    let on_toggle_terminal = move || show_terminal.update(|value| *value = !*value);

    let _ = leptos::prelude::window_event_listener(
        leptos::ev::keydown,
        move |event: web_sys::KeyboardEvent| {
            if crate::components::modal::modal_is_open() {
                return;
            }
            if event.key() == "Tab"
                && event.shift_key()
                && !event.ctrl_key()
                && !event.meta_key()
                && !event.alt_key()
                && event
                    .target()
                    .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                    .is_some_and(|element| element.class_list().contains("composer-input"))
            {
                event.prevent_default();
                let mode = super::approvals::current_mode(chat);
                select_approval_mode.run(mode.next());
                return;
            }
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
    Some(crate::text::editor_context(
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
    let history_loaded = RwSignal::new(None::<u64>);
    let history_pending = RwSignal::new(None::<String>);
    let history_saving = RwSignal::new(false);
    let history_imported = RwSignal::new(false);
    let session_pending = StoredValue::new(std::collections::HashMap::<Option<i64>, i64>::new());
    let session_saving = RwSignal::new(false);
    let resize_listener = window_event_listener(leptos::ev::resize, move |_| {
        layout.fit(viewport_width());
    });
    on_cleanup(move || resize_listener.remove());
    Effect::new(move |_| {
        let generation = auth.generation.get();
        history_loaded.set(None);
        history_pending.set(None);
        history_saving.set(false);
        history_imported.set(false);
        session_pending.set_value(Default::default());
        session_saving.set(false);
        chat.last_sessions.set(Default::default());
        chat.last_chat_session.set(None);
        let Some(user) = auth.user.get() else {
            return;
        };
        spawn_local(async move {
            let health_result = api.with_value(Clone::clone).health().await;
            if auth.generation.get_untracked() != generation {
                return;
            }
            let backend_ok = match health_result {
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
            if !backend_ok || auth.generation.get_untracked() != generation {
                return;
            }

            // Snapshot before the request so later saves survive even if the wall clock moves back.
            let candidates = crate::idb::orphan_candidates(user.id).await;
            if auth.generation.get_untracked() != generation {
                return;
            }
            let backend = api.with_value(Clone::clone);
            let (
                settings_result,
                projects_result,
                connections_result,
                sessions_result,
                prompts_result,
                model_setup_result,
            ) = futures::join!(
                backend.get_settings(),
                backend.list_projects(),
                backend.list_connections(),
                backend.list_sessions(),
                backend.list_system_prompts(),
                backend.model_setup(),
            );
            if auth.generation.get_untracked() != generation {
                return;
            }
            if let Ok(setup) = model_setup_result {
                settings.model_setup.set(setup);
            }
            let mut stored_tab_ids = Vec::<i64>::new();
            let mut stored_active_project = None;
            let mut chat_was_active = false;
            if let Ok(values) = settings_result {
                chat.approval_mode.set(
                    values
                        .iter()
                        .filter_map(|(key, value)| {
                            Some((
                                key.strip_prefix("approval_mode_")?.parse().ok()?,
                                serde_json::from_str(value).ok()?,
                            ))
                        })
                        .collect(),
                );
                chat.draft_approval_mode.set(
                    values
                        .get("draft_approval_mode")
                        .and_then(|value| serde_json::from_str(value).ok())
                        .unwrap_or(openwebide_core::ApprovalMode::NEW_SESSION),
                );

                chat.last_chat_session.set(
                    values
                        .get("last_chat_session")
                        .and_then(|value| value.parse().ok()),
                );
                chat.last_sessions.set(
                    values
                        .iter()
                        .filter_map(|(key, value)| {
                            Some((
                                key.strip_prefix("last_session_")?.parse().ok()?,
                                value.parse().ok()?,
                            ))
                        })
                        .collect(),
                );
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
                chat_was_active = values.get("active_project").is_some_and(String::is_empty);
                if let Some(id) = values
                    .get("active_project")
                    .and_then(|value| value.parse::<i64>().ok())
                {
                    stored_active_project = Some(id);
                }
                let mut widths = [
                    layout.sidebar_width.get_untracked(),
                    layout.tree_width.get_untracked(),
                    layout.chat_width.get_untracked(),
                ];
                for (i, key) in [
                    "panel_sidebar_width",
                    "panel_tree_width",
                    "panel_chat_width",
                ]
                .iter()
                .enumerate()
                {
                    if let Some(width) =
                        values.get(*key).and_then(|value| value.parse::<f64>().ok())
                    {
                        widths[i] = width;
                    }
                }
                layout.restore_panels(values.get(crate::state::layout::PANEL_VISIBILITY_KEY));
                let [sidebar, tree, chat_width] = crate::state::layout::fit_visible_panels(
                    viewport_width(),
                    widths,
                    layout.visible_panels.get_untracked(),
                );
                layout.sidebar_width.set(sidebar);
                layout.tree_width.set(tree);
                layout.chat_width.set(chat_width);

                let mut history = values
                    .get("prompt_history")
                    .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
                    .unwrap_or_default();
                // Legacy browser data is consumed only when the account has no history setting.
                if let Some(storage) =
                    web_sys::window().and_then(|window| window.local_storage().ok().flatten())
                {
                    if !values.contains_key("prompt_history")
                        && let Some(entries) = storage
                            .get_item("owide-prompt-history")
                            .ok()
                            .flatten()
                            .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
                    {
                        for entry in entries {
                            crate::history::push_history(&mut history, entry);
                        }
                    }
                    if values.contains_key("prompt_history") {
                        let _ = storage.remove_item("owide-prompt-history");
                    } else {
                        history_imported.set(true);
                    }
                    let _ = storage.remove_item("owide-theme");
                }
                chat.prompt_history.set(history);
                history_loaded.set(Some(generation));
            }

            if let Ok(connections) = connections_result {
                settings.connections.set(connections);
            }
            if let Ok(project_list) = projects_result {
                let project_ids = project_list
                    .iter()
                    .map(|project| project.id)
                    .collect::<Vec<_>>();
                projects.projects.set(project_list);
                spawn_local(async move {
                    let result = match candidates {
                        Ok(candidates) => {
                            crate::idb::delete_orphan_handles(
                                user.id,
                                &candidates,
                                &project_ids,
                                move || auth.generation.try_get_untracked() == Some(generation),
                            )
                            .await
                        }
                        Err(error) => Err(error),
                    };
                    if let Err(error) = result {
                        leptos::logging::warn!("Local folder cleanup failed: {error}");
                    }
                });
            }
            for project in projects
                .projects
                .get()
                .into_iter()
                .filter(|project| project.mode == WorkspaceMode::Local)
            {
                let handle_result = crate::idb::load_handle(project.id).await;
                if auth.generation.get_untracked() != generation {
                    return;
                }
                if let Ok(Some(handle)) = handle_result {
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
                && !chat_was_active
                && let Some(first) = all_projects.first()
            {
                restored_tab_ids.push(first.id);
            }
            projects.open_tab_ids.set(restored_tab_ids.clone());
            let active_project = if chat_was_active {
                None
            } else {
                stored_active_project
                    .filter(|id| restored_tab_ids.contains(id))
                    .or_else(|| restored_tab_ids.first().copied())
            };
            if let Some(project_id) = active_project {
                select_project.run(project_id);
            } else {
                chat.restore_chat_session();
            }
            projects.projects_loaded.set(true);
        });
    });

    Effect::new(move |_| {
        let generation = auth.generation.get();
        if auth.user.get().is_none()
            || history_loaded.get() != Some(generation)
            || !projects.projects_loaded.get()
        {
            return;
        }
        let project_id = projects.active_project.get();
        let Some(session_id) = chat.active_session.get() else {
            return;
        };
        let remembered = match project_id {
            Some(id) => chat
                .last_sessions
                .with_untracked(|sessions| sessions.get(&id).copied()),
            None => chat.last_chat_session.get_untracked(),
        };
        if !chat.sessions.with(|sessions| {
            sessions
                .iter()
                .any(|session| session.id == session_id && session.project_id == project_id)
        }) || remembered == Some(session_id)
        {
            return;
        }
        if let Some(id) = project_id {
            chat.last_sessions.update(|sessions| {
                sessions.insert(id, session_id);
            });
        } else {
            chat.last_chat_session.set(Some(session_id));
        }
        session_pending.update_value(|pending| {
            pending.insert(project_id, session_id);
        });
        if session_saving.get_untracked() {
            return;
        }
        session_saving.set(true);
        spawn_local(async move {
            loop {
                if auth.generation.try_get_untracked() != Some(generation) {
                    return;
                }
                let next = session_pending.with_value(|pending| {
                    pending
                        .iter()
                        .next()
                        .map(|(project, session)| (*project, *session))
                });
                let Some((project_id, session_id)) = next else {
                    break;
                };
                session_pending.update_value(|pending| {
                    pending.remove(&project_id);
                });
                let key = project_id.map_or_else(
                    || "last_chat_session".to_string(),
                    |id| format!("last_session_{id}"),
                );
                if let Err(error) = api
                    .with_value(Clone::clone)
                    .set_setting(&key, &session_id.to_string())
                    .await
                    && auth.generation.try_get_untracked() == Some(generation)
                {
                    chat.error.set(Some(format!(
                        "Could not save the last used session: {error}"
                    )));
                }
            }
            session_saving.set(false);
        });
    });

    Effect::new(move |_| {
        let generation = auth.generation.get();
        if auth.user.get().is_none() || history_loaded.get() != Some(generation) {
            return;
        }
        let history = chat.prompt_history.get();
        if let Ok(json) = serde_json::to_string(&history) {
            history_pending.set(Some(json));
            if history_saving.get_untracked() {
                return;
            }
            history_saving.set(true);
            spawn_local(async move {
                loop {
                    if auth.generation.get_untracked() != generation {
                        return;
                    }
                    let mut next = None;
                    history_pending.update(|pending| next = pending.take());
                    let Some(json) = next else {
                        break;
                    };
                    if api
                        .with_value(Clone::clone)
                        .set_setting("prompt_history", &json)
                        .await
                        .is_ok()
                        && auth.generation.get_untracked() == generation
                        && history_imported.get_untracked()
                    {
                        if let Some(storage) = web_sys::window()
                            .and_then(|window| window.local_storage().ok().flatten())
                        {
                            let _ = storage.remove_item("owide-prompt-history");
                        }
                        history_imported.set(false);
                    }
                }
                history_saving.set(false);
            });
        }
    });

    Effect::new(move |_| {
        if !projects.projects_loaded.get() {
            return;
        }
        let ids = projects.open_tab_ids.get();
        if let Ok(json) = serde_json::to_string(&ids) {
            spawn_local(async move {
                let _ = api
                    .with_value(Clone::clone)
                    .set_setting("open_tabs", &json)
                    .await;
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
            let _ = api
                .with_value(Clone::clone)
                .set_setting("active_project", &value)
                .await;
        });
    });
}

fn viewport_width() -> f64 {
    web_sys::window()
        .and_then(|window| window.inner_width().ok())
        .and_then(|value| value.as_f64())
        .unwrap_or(1200.0)
}
