use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::backend::Api;
use crate::state::{
    settings::{SettingsState, Theme},
    ui::{ConfirmRequest, UiState},
};

pub struct SettingsActionContext {
    pub api: Api,
    pub settings: SettingsState,
    pub ui: UiState,
}

#[derive(Clone, Copy)]
pub struct SettingsActions {
    pub on_new_prompt: Callback<()>,
    pub on_edit_prompt: Callback<i64>,
    pub on_cancel_prompt: Callback<()>,
    pub on_save_prompt: Callback<()>,
    pub on_delete_prompt: Callback<i64>,
    pub on_new_connection: Callback<()>,
    pub on_edit_connection: Callback<i64>,
    pub on_cancel_connection: Callback<()>,
    pub on_delete_connection: Callback<i64>,
    pub on_open_settings: Callback<()>,
    pub on_set_theme: Callback<Theme>,
    pub on_set_editor_preferences: Callback<openwebide_core::editor::EditorPreferences>,
    pub on_set_notifications: Callback<bool>,
    pub on_set_default_connection: Callback<Option<i64>>,
    pub on_set_default_prompt: Callback<Option<i64>>,
    pub on_set_bridge_url: Callback<String>,
}

pub fn build_settings_actions(context: SettingsActionContext) -> SettingsActions {
    let SettingsActionContext { api, settings, ui } = context;
    install_theme_effect(settings);
    let auth = expect_context::<crate::state::auth::AuthState>();

    let on_new_prompt = Callback::new(move |()| {
        settings.show_prompt_form.set(true);
        settings.prompt_edit_id.set(None);
        settings.prompt_name.set(String::new());
        settings.prompt_content.set(String::new());
    });

    let on_edit_prompt = Callback::new(move |id: i64| {
        let Some(prompt) = settings
            .system_prompts
            .with(|items| items.iter().find(|prompt| prompt.id == id).cloned())
        else {
            return;
        };
        settings.prompt_edit_id.set(Some(id));
        settings.prompt_name.set(prompt.name);
        settings.prompt_content.set(prompt.content);
        settings.show_prompt_form.set(true);
    });

    let on_cancel_prompt = Callback::new(move |()| {
        settings.show_prompt_form.set(false);
    });

    let on_save_prompt = Callback::new(move |()| {
        let name = settings.prompt_name.get().trim().to_string();
        if name.is_empty() {
            ui.notify("Prompt name is required.");
            return;
        }
        let content = settings.prompt_content.get();
        let edit_id = settings.prompt_edit_id.get();
        let account = auth.generation.get_untracked();
        ui.clear_toast();
        spawn_local(async move {
            if auth.generation.try_get_untracked() != Some(account) {
                return;
            }
            let result = match edit_id {
                Some(id) => {
                    api.with_value(Clone::clone)
                        .update_system_prompt(id, &name, &content)
                        .await
                }
                None => {
                    api.with_value(Clone::clone)
                        .create_system_prompt(&name, &content)
                        .await
                }
            };
            if auth.generation.try_get_untracked() != Some(account) {
                return;
            }
            match result {
                Ok(prompt) => {
                    settings.system_prompts.update(|prompts| match edit_id {
                        Some(id) => {
                            if let Some(existing) = prompts.iter_mut().find(|item| item.id == id) {
                                *existing = prompt;
                            }
                        }
                        None => prompts.push(prompt),
                    });
                    settings.show_prompt_form.set(false);
                }
                Err(error) => ui.notify(error),
            }
        });
    });

    let on_delete_prompt = Callback::new(move |id: i64| {
        let account = auth.generation.get_untracked();
        ui.set_confirm(ConfirmRequest {
            title: "Delete system prompt".to_string(),
            message: "Delete this system prompt?".to_string(),
            confirm_label: "Delete".to_string(),
            action: Callback::new(move |()| {
                if auth.generation.try_get_untracked() != Some(account) {
                    return;
                }
                spawn_local(async move {
                    if auth.generation.try_get_untracked() != Some(account) {
                        return;
                    }
                    let result = api.with_value(Clone::clone).delete_system_prompt(id).await;
                    if auth.generation.try_get_untracked() != Some(account) {
                        return;
                    }
                    if let Err(error) = result {
                        ui.notify(error);
                        return;
                    }
                    settings
                        .system_prompts
                        .update(|prompts| prompts.retain(|prompt| prompt.id != id));
                    if settings.default_prompt.get_untracked() == Some(id) {
                        settings.default_prompt.set(None);
                    }
                });
            }),
        });
    });

    let on_new_connection = Callback::new(move |()| settings.begin_model_setup(None));
    let on_edit_connection = Callback::new(move |id: i64| settings.begin_model_setup(Some(id)));

    let on_cancel_connection = Callback::new(move |()| {
        settings.show_conn_form.set(false);
    });

    let on_delete_connection = Callback::new(move |id: i64| {
        ui.set_confirm(ConfirmRequest {
            title: "Delete connection".to_string(),
            message: "Delete this connection? Sessions using it will lose their LLM connection."
                .to_string(),
            confirm_label: "Delete".to_string(),
            action: Callback::new(move |()| {
                spawn_local(async move {
                    if let Err(error) = api.with_value(Clone::clone).delete_connection(id).await {
                        ui.notify(error);
                        return;
                    }
                    settings
                        .connections
                        .update(|connections| connections.retain(|connection| connection.id != id));
                });
            }),
        });
    });

    let notifications = crate::notifications::RunNotifications::from_context();
    let on_set_notifications = Callback::new(move |enabled: bool| {
        use crate::notifications::NotificationPermission;
        if notifications.configuring.get_untracked() {
            return;
        }
        notifications.refresh_permission();
        let permission = notifications.permission.get_untracked();
        if enabled
            && matches!(
                permission,
                NotificationPermission::Unsupported | NotificationPermission::Denied
            )
        {
            ui.notify(
                "Allow notifications in this browser's site settings, using HTTPS or localhost.",
            );
            return;
        }
        let account = auth.generation.get_untracked();
        notifications.revision.update_value(|value| *value += 1);
        let revision = notifications.revision.get_value();
        notifications.configuring.set(true);
        let requested = if enabled && permission != NotificationPermission::Granted {
            Some(notifications.host().request_permission())
        } else {
            None
        };
        spawn_local(async move {
            let current = || {
                auth.generation.try_get_untracked() == Some(account)
                    && notifications.revision.try_get_value() == Some(revision)
            };
            if let Some(requested) = requested {
                let result = requested.await;
                if !current() {
                    return;
                }
                match result {
                    Ok(NotificationPermission::Granted) => notifications
                        .permission
                        .set(NotificationPermission::Granted),
                    Ok(permission) => {
                        notifications.permission.set(permission);
                        notifications.configuring.set(false);
                        ui.notify("Notifications were not enabled. You can allow them in this browser's site settings.");
                        return;
                    }
                    Err(error) => {
                        notifications.configuring.set(false);
                        ui.notify(format!("Could not enable notifications: {error}"));
                        return;
                    }
                }
            }
            if !current() {
                return;
            }
            let result = api
                .with_value(Clone::clone)
                .set_setting(
                    "browser_notifications",
                    if enabled { "true" } else { "false" },
                )
                .await;
            if !current() {
                return;
            }
            notifications.configuring.set(false);
            match result {
                Ok(()) => {
                    settings.browser_notifications.set(enabled);
                    if !enabled {
                        notifications.host().close();
                    }
                }
                Err(error) => ui.notify(format!("Could not save notification preference: {error}")),
            }
        });
    });
    let on_open_settings = Callback::new(move |()| {
        notifications.refresh_permission();
        settings.show_settings.set(true);
    });

    let editor_auth = expect_context::<crate::state::auth::AuthState>();
    let pending_editor_preferences =
        StoredValue::new(None::<openwebide_core::editor::EditorPreferences>);
    let saving_editor_preferences = StoredValue::new(None::<u64>);
    let confirmed_editor_preferences =
        StoredValue::new(None::<openwebide_core::editor::EditorPreferences>);
    Effect::new(move |_| {
        editor_auth.generation.track();
        pending_editor_preferences.set_value(None);
        confirmed_editor_preferences.set_value(None);
    });
    let on_set_editor_preferences = Callback::new(
        move |preferences: openwebide_core::editor::EditorPreferences| {
            let preferences = preferences.normalized();
            if confirmed_editor_preferences.get_value().is_none() {
                confirmed_editor_preferences
                    .set_value(Some(settings.editor_preferences.get_untracked()));
            }
            settings.editor_preferences.set(preferences);
            settings
                .editor_preference_revision
                .update(|revision| *revision += 1);
            pending_editor_preferences.set_value(Some(preferences));
            let account = editor_auth.generation.get_untracked();
            if saving_editor_preferences.get_value() == Some(account) {
                return;
            }
            saving_editor_preferences.set_value(Some(account));
            spawn_local(async move {
                loop {
                    if editor_auth.generation.try_get_untracked() != Some(account) {
                        return;
                    }
                    let mut preferences = None;
                    pending_editor_preferences.update_value(|pending| preferences = pending.take());
                    let Some(preferences) = preferences else {
                        break;
                    };
                    let value =
                        serde_json::to_string(&preferences).expect("editor preferences serialize");
                    let result = api
                        .with_value(Clone::clone)
                        .set_setting("editor_preferences", &value)
                        .await;
                    if editor_auth.generation.try_get_untracked() != Some(account) {
                        return;
                    }
                    match result {
                        Ok(()) => confirmed_editor_preferences.set_value(Some(preferences)),
                        Err(error) => {
                            if pending_editor_preferences.get_value().is_none() {
                                settings.editor_preferences.set(
                                    confirmed_editor_preferences.get_value().unwrap_or_default(),
                                );
                                ui.notify(format!("Could not save editor preferences: {error}"));
                            }
                        }
                    }
                }
                if saving_editor_preferences.try_get_value() == Some(Some(account)) {
                    saving_editor_preferences.set_value(None);
                }
            });
        },
    );

    let on_set_theme = Callback::new(move |theme: Theme| {
        settings.theme.set(theme);
        spawn_local(async move {
            if let Err(error) = api
                .with_value(Clone::clone)
                .set_setting("theme", theme.as_str())
                .await
            {
                ui.notify(error);
            }
        });
    });

    let on_set_default_connection = Callback::new(move |value: Option<i64>| {
        settings.default_connection.set(value);
        let value = value.map(|id| id.to_string()).unwrap_or_default();
        spawn_local(async move {
            if let Err(error) = api
                .with_value(Clone::clone)
                .set_setting("default_connection", &value)
                .await
            {
                ui.notify(error);
            }
        });
    });

    let on_set_default_prompt = Callback::new(move |value: Option<i64>| {
        settings.default_prompt.set(value);
        let value = value.map(|id| id.to_string()).unwrap_or_default();
        spawn_local(async move {
            if let Err(error) = api
                .with_value(Clone::clone)
                .set_setting("default_prompt", &value)
                .await
            {
                ui.notify(error);
            }
        });
    });

    let on_set_bridge_url = Callback::new(move |url: String| {
        settings.bridge_url.set(url.clone());
        spawn_local(async move {
            let _ = api
                .with_value(Clone::clone)
                .set_setting("bridge_url", &url)
                .await;
        });
    });

    SettingsActions {
        on_new_prompt,
        on_edit_prompt,
        on_cancel_prompt,
        on_save_prompt,
        on_delete_prompt,
        on_new_connection,
        on_edit_connection,
        on_cancel_connection,
        on_delete_connection,
        on_open_settings,
        on_set_theme,
        on_set_editor_preferences,
        on_set_notifications,
        on_set_default_connection,
        on_set_default_prompt,
        on_set_bridge_url,
    }
}

fn install_theme_effect(settings: SettingsState) {
    let theme = settings.theme;
    Effect::new(move |_| {
        if let Some(document) = web_sys::window().and_then(|window| window.document())
            && let Some(root) = document.document_element()
        {
            let _ = root.set_attribute("data-theme", effective_theme(theme.get()));
        }
    });

    if let Some(media_query) = web_sys::window().and_then(|window| {
        window
            .match_media("(prefers-color-scheme: light)")
            .ok()
            .flatten()
    }) {
        let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            if theme.get() != Theme::System {
                return;
            }
            if let Some(document) = web_sys::window().and_then(|window| window.document())
                && let Some(root) = document.document_element()
            {
                let _ = root.set_attribute("data-theme", effective_theme(Theme::System));
            }
        });
        let _ = media_query
            .add_event_listener_with_callback("change", callback.as_ref().unchecked_ref());
        StoredValue::new_local(callback);
    }
}

fn effective_theme(preference: Theme) -> &'static str {
    if preference == Theme::System {
        let prefers_light = web_sys::window()
            .and_then(|window| {
                window
                    .match_media("(prefers-color-scheme: light)")
                    .ok()
                    .flatten()
            })
            .is_some_and(|query| query.matches());
        if prefers_light { "light" } else { "dark" }
    } else {
        preference.as_str()
    }
}

/// Serialize autosaves in the user-owned settings store so closing a dialog cannot
/// discard a queued choice and an older response cannot replace a newer choice.
pub fn set_model_defaults(
    api: Api,
    settings: SettingsState,
    auth: crate::state::auth::AuthState,
    ui: UiState,
    defaults: openwebide_core::ModelDefaults,
) {
    let account = auth.generation.get_untracked();
    settings
        .model_setup
        .update(|setup| setup.defaults = defaults.clone());
    if let Some(primary) = &defaults.primary {
        settings.default_connection.set(Some(primary.server_id));
    }
    settings
        .model_defaults_revision
        .update(|revision| *revision += 1);
    settings
        .pending_model_defaults
        .set_value(Some((account, defaults)));
    settings.model_defaults_error.set(None);
    if settings.model_defaults_saving.get_untracked() == Some(account) {
        return;
    }
    settings.model_defaults_saving.set(Some(account));
    spawn_local(async move {
        loop {
            if auth.generation.try_get_untracked() != Some(account) {
                return;
            }
            let mut pending = None;
            settings.pending_model_defaults.update_value(|value| {
                if value.as_ref().is_some_and(|(epoch, _)| *epoch == account) {
                    pending = value.take();
                }
            });
            let Some((_, defaults)) = pending else {
                break;
            };
            let result = api
                .with_value(Clone::clone)
                .save_model_defaults(&defaults)
                .await;
            if auth.generation.try_get_untracked() != Some(account)
                || settings.model_defaults_saving.try_get_untracked() != Some(Some(account))
            {
                return;
            }
            if settings.pending_model_defaults.with_value(Option::is_some) {
                continue;
            }
            match result {
                Ok(saved) => settings
                    .model_setup
                    .update(|setup| setup.defaults = saved.defaults),
                Err(message) => {
                    settings
                        .model_defaults_error
                        .set(Some((account, message.clone())));
                    ui.notify(format!("Could not save model defaults: {message}"));
                }
            }
        }
        settings.model_defaults_saving.set(None);
    });
}
