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
    pub on_set_default_connection: Callback<Option<i64>>,
    pub on_set_default_prompt: Callback<Option<i64>>,
    pub on_set_bridge_url: Callback<String>,
}

pub fn build_settings_actions(context: SettingsActionContext) -> SettingsActions {
    let SettingsActionContext { api, settings, ui } = context;
    install_theme_effect(settings);

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
        ui.clear_toast();
        spawn_local(async move {
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
        ui.set_confirm(ConfirmRequest {
            title: "Delete system prompt".to_string(),
            message: "Delete this system prompt?".to_string(),
            confirm_label: "Delete".to_string(),
            action: Callback::new(move |()| {
                spawn_local(async move {
                    if let Err(error) = api.with_value(Clone::clone).delete_system_prompt(id).await
                    {
                        ui.notify(error);
                        return;
                    }
                    settings
                        .system_prompts
                        .update(|prompts| prompts.retain(|prompt| prompt.id != id));
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

    let on_open_settings = Callback::new(move |()| {
        settings.show_settings.set(true);
    });

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
