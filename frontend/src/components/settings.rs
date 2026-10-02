use super::modal::Modal;
use crate::state::settings::{SettingsState, Theme};
use leptos::prelude::*;
use web_sys::wasm_bindgen::JsCast;

/// The settings dialog: theme, and the connection / system prompt used as
/// defaults for new sessions.
#[component]
pub fn Settings(
    on_set_theme: Callback<Theme>,
    on_set_default_connection: Callback<Option<i64>>,
    on_set_default_prompt: Callback<Option<i64>>,
    on_set_bridge_url: Callback<String>,
) -> impl IntoView {
    let settings = expect_context::<SettingsState>();
    let theme = settings.theme.read_only();
    let default_connection = settings.default_connection.read_only();
    let default_prompt = settings.default_prompt.read_only();
    let connections = settings.connections.read_only();
    let system_prompts = settings.system_prompts.read_only();
    let bridge_url = settings.bridge_url.read_only();
    let on_close = Callback::new(move |()| settings.show_settings.set(false));
    let conn_ref = NodeRef::<leptos::html::Select>::new();
    let prompt_ref = NodeRef::<leptos::html::Select>::new();

    // Leptos 0.8 has no reactive `value` for <select>, so set the DOM value
    // directly whenever the chosen id or the option list changes.
    Effect::new(move || {
        let value = default_connection
            .get()
            .map(|id| id.to_string())
            .unwrap_or_default();
        connections.track();
        if let Some(el) = conn_ref.get() {
            el.set_value(&value);
        }
    });
    Effect::new(move || {
        let value = default_prompt
            .get()
            .map(|id| id.to_string())
            .unwrap_or_default();
        system_prompts.track();
        if let Some(el) = prompt_ref.get() {
            el.set_value(&value);
        }
    });

    let (pairing_token, set_pairing_token) = leptos::prelude::signal(String::new());
    Effect::new(move || {
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(Some(pt)) = crate::idb::get_bridge_pairing_token().await {
                set_pairing_token.set(pt);
            }
        });
    });

    view! {
        <Modal title=Signal::derive(|| "Settings".to_string()) on_close=on_close>
            <div class="modal-body">
                <div class="setting-row">
                    <span class="setting-label">"Theme"</span>
                    <div class="mode-picker">
                        <label
                            class=move || {
                                if theme.get() == Theme::System {
                                    "mode-opt active".to_string()
                                } else {
                                    "mode-opt".to_string()
                                }
                            }
                        >
                            <input
                                type="radio"
                                name="theme"
                                checked=move || theme.get() == Theme::System
                                on:click=move |_| on_set_theme.run(Theme::System)
                            />
                            "System"
                        </label>
                        <label
                            class=move || {
                                if theme.get() == Theme::Dark {
                                    "mode-opt active".to_string()
                                } else {
                                    "mode-opt".to_string()
                                }
                            }
                        >
                            <input
                                type="radio"
                                name="theme"
                                checked=move || theme.get() == Theme::Dark
                                on:click=move |_| on_set_theme.run(Theme::Dark)
                            />
                            "Dark"
                        </label>
                        <label
                            class=move || {
                                if theme.get() == Theme::Light {
                                    "mode-opt active".to_string()
                                } else {
                                    "mode-opt".to_string()
                                }
                            }
                        >
                            <input
                                type="radio"
                                name="theme"
                                checked=move || theme.get() == Theme::Light
                                on:click=move |_| on_set_theme.run(Theme::Light)
                            />
                            "Light"
                        </label>
                    </div>
                </div>

                <div class="setting-row">
                    <span class="setting-label">"Default connection"</span>
                    <select
                        class="form-input"
                        node_ref=conn_ref
                        on:change=move |e: web_sys::Event| {
                            if let Some(target) = e.target()
                                && let Some(sel) = target.dyn_ref::<web_sys::HtmlSelectElement>()
                            {
                                on_set_default_connection.run(sel.value().parse::<i64>().ok());
                            }
                        }
                    >
                        <option value="">"(none)"</option>
                        {connections
                            .get()
                            .into_iter()
                            .map(|c| {
                                let name = c.name.clone();
                                let value = c.id.to_string();
                                view! { <option value=value>{name}</option> }
                            })
                            .collect::<Vec<_>>()}
                    </select>
                </div>

                <div class="setting-row">
                    <span class="setting-label">"Default system prompt"</span>
                    <select
                        class="form-input"
                        node_ref=prompt_ref
                        on:change=move |e: web_sys::Event| {
                            if let Some(target) = e.target()
                                && let Some(sel) = target.dyn_ref::<web_sys::HtmlSelectElement>()
                            {
                                on_set_default_prompt.run(sel.value().parse::<i64>().ok());
                            }
                        }
                    >
                        <option value="">"(none)"</option>
                        {system_prompts
                            .get()
                            .into_iter()
                            .map(|p| {
                                let name = p.name.clone();
                                let value = p.id.to_string();
                                view! { <option value=value>{name}</option> }
                            })
                            .collect::<Vec<_>>()}
                    </select>
                </div>

                <div class="setting-row">
                    <span class="setting-label">"Bridge URL"</span>
                    <input
                        type="text"
                        class="form-input"
                        value=bridge_url
                        on:change=move |e: web_sys::Event| {
                            if let Some(target) = e.target()
                                && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                            {
                                on_set_bridge_url.run(input.value());
                            }
                        }
                    />
                </div>

                <div class="setting-row" style="flex-direction: column; align-items: flex-start; gap: 0.5rem;">
                    <span class="setting-label">"Bridge pairing token"</span>
                    <div style="display: flex; gap: 0.5rem; width: 100%;">
                        <input
                            type="password"
                            class="form-input"
                            style="flex: 1;"
                            value=pairing_token
                            on:change=move |e: web_sys::Event| {
                                if let Some(target) = e.target()
                                    && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                                {
                                    let val = input.value();
                                    set_pairing_token.set(val.clone());
                                    wasm_bindgen_futures::spawn_local(async move {
                                        let _ = crate::idb::set_bridge_pairing_token(&val).await;
                                    });
                                }
                            }
                        />
                        <button
                            class="btn stop"
                            on:click=move |_| {
                                set_pairing_token.set(String::new());
                                wasm_bindgen_futures::spawn_local(async move {
                                    let _ = crate::idb::delete_bridge_pairing_token().await;
                                });
                            }
                        >
                            "Clear"
                        </button>
                    </div>
                    <div class="form-hint" style="color: var(--border-subtle); font-size: 0.85rem;">
                        "Required only if your bridge daemon was started with OPENWEBIDE_BRIDGE_TOKEN to allow local execution (the laptop-companion case)."
                    </div>
                </div>

            </div>
        </Modal>
    }
}
