use leptos::prelude::*;
use openwebide_core::{Connection, SystemPrompt};
use web_sys::wasm_bindgen::JsCast;

/// The settings dialog: theme, and the connection / system prompt used as
/// defaults for new sessions.
#[component]
pub fn Settings(
    theme: ReadSignal<String>,
    default_connection: ReadSignal<Option<i64>>,
    default_prompt: ReadSignal<Option<i64>>,
    connections: ReadSignal<Vec<Connection>>,
    system_prompts: ReadSignal<Vec<SystemPrompt>>,
    on_close: Callback<()>,
    on_set_theme: Callback<String>,
    on_set_default_connection: Callback<Option<i64>>,
    on_set_default_prompt: Callback<Option<i64>>,
) -> impl IntoView {
    let conn_ref = NodeRef::<leptos::html::Select>::new();
    let prompt_ref = NodeRef::<leptos::html::Select>::new();

    // Leptos 0.8 has no reactive `value` for <select>, so set the DOM value
    // directly whenever the chosen id or the option list changes.
    Effect::new(move || {
        let value = default_connection
            .get()
            .map(|id| id.to_string())
            .unwrap_or_default();
        let _ = connections.get();
        if let Some(el) = conn_ref.get() {
            el.set_value(&value);
        }
    });
    Effect::new(move || {
        let value = default_prompt
            .get()
            .map(|id| id.to_string())
            .unwrap_or_default();
        let _ = system_prompts.get();
        if let Some(el) = prompt_ref.get() {
            el.set_value(&value);
        }
    });

    view! {
        <div class="modal-overlay" on:click=move |_| on_close.run(())>
            <div class="modal" on:click=move |e: web_sys::MouseEvent| e.stop_propagation()>
                <div class="modal-header">
                    <h2>"Settings"</h2>
                    <button
                        class="icon-btn"
                        title="Close"
                        on:click=move |_| on_close.run(())
                    >
                        "✕"
                    </button>
                </div>
                <div class="modal-body">
                    <div class="setting-row">
                        <span class="setting-label">"Theme"</span>
                        <div class="mode-picker">
                            <label
                                class=move || {
                                    if theme.get() == "dark" {
                                        "mode-opt active".to_string()
                                    } else {
                                        "mode-opt".to_string()
                                    }
                                }
                            >
                                <input
                                    type="radio"
                                    name="theme"
                                    checked=move || theme.get() == "dark"
                                    on:click=move |_| on_set_theme.run("dark".to_string())
                                />
                                "Dark"
                            </label>
                            <label
                                class=move || {
                                    if theme.get() == "light" {
                                        "mode-opt active".to_string()
                                    } else {
                                        "mode-opt".to_string()
                                    }
                                }
                            >
                                <input
                                    type="radio"
                                    name="theme"
                                    checked=move || theme.get() == "light"
                                    on:click=move |_| on_set_theme.run("light".to_string())
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
                </div>
            </div>
        </div>
    }
}
