use leptos::prelude::*;
use openwebide_core::ProviderKind;
use web_sys::wasm_bindgen::JsCast;

use crate::state::{
    chat::ChatState, layout::LayoutState, projects::ProjectsState, settings::SettingsState,
};

#[component]
pub fn Sidebar(
    on_new_connection: Callback<()>,
    on_edit_connection: Callback<i64>,
    on_save_connection: Callback<()>,
    on_cancel_connection: Callback<()>,
    on_delete_connection: Callback<i64>,
    on_select_session: Callback<i64>,
    on_new_session: Callback<()>,
    on_rename_session: Callback<i64>,
    on_delete_session: Callback<i64>,
    on_new_prompt: Callback<()>,
    on_edit_prompt: Callback<i64>,
    on_save_prompt: Callback<()>,
    on_cancel_prompt: Callback<()>,
    on_delete_prompt: Callback<i64>,
) -> impl IntoView {
    let settings = expect_context::<SettingsState>();
    let projects = expect_context::<ProjectsState>();
    let chat = expect_context::<ChatState>();
    let layout = expect_context::<LayoutState>();

    let connections = settings.connections.read_only();
    let show_conn_form = settings.show_conn_form.read_only();
    let conn_edit_id = settings.conn_edit_id.read_only();
    let conn_name = settings.conn_name.read_only();
    let set_conn_name = settings.conn_name.write_only();
    let conn_kind = settings.conn_kind.read_only();
    let set_conn_kind = settings.conn_kind.write_only();
    let conn_base_url = settings.conn_base_url.read_only();
    let set_conn_base_url = settings.conn_base_url.write_only();
    let conn_model = settings.conn_model.read_only();
    let set_conn_model = settings.conn_model.write_only();
    let conn_context_limit = settings.conn_context_limit.read_only();
    let set_conn_context_limit = settings.conn_context_limit.write_only();
    let sessions = chat.sessions.read_only();
    let active_project = projects.active_project.read_only();
    let active_session = chat.active_session.read_only();
    let system_prompts = settings.system_prompts.read_only();
    let show_prompt_form = settings.show_prompt_form.read_only();
    let prompt_edit_id = settings.prompt_edit_id.read_only();
    let prompt_name = settings.prompt_name.read_only();
    let set_prompt_name = settings.prompt_name.write_only();
    let prompt_content = settings.prompt_content.read_only();
    let set_prompt_content = settings.prompt_content.write_only();
    let sidebar_width = layout.sidebar_width.read_only();

    // Leptos has no `value` attribute for <textarea>, so the form's content is
    // synced into the DOM node when the form appears.
    let prompt_ta = NodeRef::<leptos::html::Textarea>::new();
    Effect::new(move || {
        if show_prompt_form.get()
            && let Some(ta) = prompt_ta.get()
        {
            ta.set_value(&prompt_content.get());
        }
    });

    view! {
        <aside
            class="sidebar"
            style=move || format!("width: {}px; flex: none;", sidebar_width.get())
        >
            // -- sessions (scoped to the active project) ---------------------
            <div class="sidebar-section">
                <div class="section-header">
                    <h2>"Sessions"</h2>
                    <button
                        class="icon-btn"
                        title="New chat"
                        disabled=move || active_project.get().is_none()
                        on:click=move |_| on_new_session.run(())
                    >
                        "+"
                    </button>
                </div>
                <Show
                    when=move || active_project.get().is_some()
                    fallback=move || {
                        view! { <p class="empty">"Open a project to start a session."</p> }
                    }
                >
                    <For
                        each=move || {
                            let active = active_project.get();
                            sessions
                                .get()
                                .into_iter()
                                .filter(|s| s.project_id == active)
                                .collect::<Vec<_>>()
                        }
                        key=|s| (s.id, s.name.clone())
                        children=move |s| {
                            view! {
                                <div
                                    class=move || {
                                        if active_session.get() == Some(s.id) {
                                            "session active".to_string()
                                        } else {
                                            "session".to_string()
                                        }
                                    }
                                >
                                    <span
                                        class="session-name"
                                        on:click=move |_| on_select_session.run(s.id)
                                    >
                                        {s.name}
                                    </span>
                                    <span class="session-actions">
                                        <button
                                            class="icon-btn"
                                            title="Rename"
                                            on:click=move |_| on_rename_session.run(s.id)
                                        >
                                            "✎"
                                        </button>
                                        <button
                                            class="icon-btn"
                                            title="Delete"
                                            on:click=move |_| on_delete_session.run(s.id)
                                        >
                                            "✕"
                                        </button>
                                    </span>
                                </div>
                            }
                        }
                    />
                    <Show
                        when=move || {
                            let active = active_project.get();
                            sessions.get().iter().all(|s| s.project_id != active)
                        }
                        fallback=|| ()
                    >
                        <p class="empty">"No sessions yet — start chatting to create one."</p>
                    </Show>
                </Show>
            </div>

            // -- connections --------------------------------------------------
            <div class="sidebar-section">
                <div class="section-header">
                    <h2>"Connections"</h2>
                    <button
                        class="icon-btn"
                        title="New connection"
                        on:click=move |_| on_new_connection.run(())
                    >
                        "+"
                    </button>
                </div>
                <Show when=move || show_conn_form.get() fallback=|| ()>
                    <div class="new-project-form">
                        <input
                            type="text"
                            class="form-input"
                            placeholder="Connection name"
                            value=move || conn_name.get()
                            on:input=move |e: web_sys::Event| {
                                if let Some(target) = e.target()
                                    && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                                {
                                    set_conn_name.set(input.value());
                                }
                            }
                        />
                        <div class="mode-picker">
                            <label
                                class=move || {
                                    if conn_kind.get() == ProviderKind::Ollama {
                                        "mode-opt active".to_string()
                                    } else {
                                        "mode-opt".to_string()
                                    }
                                }
                            >
                                <input
                                    type="radio"
                                    name="conn-kind"
                                    checked=move || conn_kind.get() == ProviderKind::Ollama
                                    on:click=move |_| set_conn_kind.set(ProviderKind::Ollama)
                                />
                                "Ollama"
                            </label>
                            <label
                                class=move || {
                                    if conn_kind.get() == ProviderKind::LlamaCpp {
                                        "mode-opt active".to_string()
                                    } else {
                                        "mode-opt".to_string()
                                    }
                                }
                            >
                                <input
                                    type="radio"
                                    name="conn-kind"
                                    checked=move || conn_kind.get() == ProviderKind::LlamaCpp
                                    on:click=move |_| set_conn_kind.set(ProviderKind::LlamaCpp)
                                />
                                "llama.cpp"
                            </label>
                        </div>
                        <input
                            type="text"
                            class="form-input"
                            placeholder="Base URL (e.g. http://localhost:11434)"
                            value=move || conn_base_url.get()
                            on:input=move |e: web_sys::Event| {
                                if let Some(target) = e.target()
                                    && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                                {
                                    set_conn_base_url.set(input.value());
                                }
                            }
                        />
                        <input
                            type="text"
                            class="form-input"
                            placeholder="Model (optional)"
                            value=move || conn_model.get()
                            on:input=move |e: web_sys::Event| {
                                if let Some(target) = e.target()
                                    && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                                {
                                    set_conn_model.set(input.value());
                                }
                            }
                        />
                        <input
                            type="text"
                            class="form-input"
                            inputmode="numeric"
                            placeholder="Context limit in tokens (optional)"
                            value=move || conn_context_limit.get()
                            on:input=move |e: web_sys::Event| {
                                if let Some(target) = e.target()
                                    && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                                {
                                    set_conn_context_limit.set(input.value());
                                }
                            }
                        />
                        <div class="form-hint">
                            {move || match conn_kind.get() {
                                ProviderKind::Ollama => {
                                    "Sent to Ollama as num_ctx on every request. Leave blank to use the model's default."
                                }
                                ProviderKind::LlamaCpp => {
                                    "Display only: llama.cpp's context size is fixed when llama-server starts (-c / --ctx-size). Set this to match it, or leave blank to read it from the server."
                                }
                            }}
                        </div>
                        <Show when=move || conn_edit_id.get().is_some_and(|id| connections.get().iter().any(|connection| connection.id == id && connection.tool_stream_unsupported))>
                            <div class="form-hint">"Streaming tool calls disabled — this server rejected them. Changing the server URL or provider kind re-checks."</div>
                        </Show>
                        <div class="form-actions">
                            <button
                                class="btn send"
                                on:click=move |_| on_save_connection.run(())
                            >
                                {move || {
                                    if conn_edit_id.get().is_some() {
                                        "Save".to_string()
                                    } else {
                                        "Create".to_string()
                                    }
                                }}
                            </button>
                            <button
                                class="btn"
                                on:click=move |_| on_cancel_connection.run(())
                            >
                                "Cancel"
                            </button>
                        </div>
                    </div>
                </Show>
                <For
                    each=move || connections.get()
                    key=|c| (c.id, c.name.clone(), c.kind)
                    children=move |c| {
                        let id = c.id;
                        let name = c.name.clone();
                        let kind = c.kind;
                        view! {
                            <div class="connection">
                                <span class="conn-name">{name}</span>
                                <span class="conn-kind">{kind.as_str()}</span>
                                <span class="conn-actions">
                                    <button
                                        class="icon-btn"
                                        title="Edit"
                                        on:click=move |_| on_edit_connection.run(id)
                                    >
                                        "✎"
                                    </button>
                                    <button
                                        class="icon-btn"
                                        title="Delete"
                                        on:click=move |_| on_delete_connection.run(id)
                                    >
                                        "✕"
                                    </button>
                                </span>
                            </div>
                        }
                    }
                />
                <Show when=move || connections.get().is_empty() fallback=|| ()>
                    <p class="empty">"No connections yet — add one."</p>
                </Show>
            </div>

            // -- system prompts ----------------------------------------------
            <div class="sidebar-section">
                <div class="section-header">
                    <h2>"System prompts"</h2>
                    <button
                        class="icon-btn"
                        title="New prompt"
                        on:click=move |_| on_new_prompt.run(())
                    >
                        "+"
                    </button>
                </div>
                <Show when=move || show_prompt_form.get() fallback=|| ()>
                    <div class="new-project-form">
                        <input
                            type="text"
                            class="form-input"
                            placeholder="Prompt name"
                            value=move || prompt_name.get()
                            on:input=move |e: web_sys::Event| {
                                if let Some(target) = e.target()
                                    && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                                {
                                    set_prompt_name.set(input.value());
                                }
                            }
                        />
                        <textarea
                            class="form-input prompt-content"
                            placeholder="System prompt content"
                            rows="6"
                            node_ref=prompt_ta
                            on:input=move |e: web_sys::Event| {
                                if let Some(target) = e.target()
                                    && let Some(ta) = target.dyn_ref::<web_sys::HtmlTextAreaElement>()
                                {
                                    set_prompt_content.set(ta.value());
                                }
                            }
                        />
                        <div class="form-actions">
                            <button
                                class="btn send"
                                on:click=move |_| on_save_prompt.run(())
                            >
                                {move || {
                                    if prompt_edit_id.get().is_some() {
                                        "Save".to_string()
                                    } else {
                                        "Create".to_string()
                                    }
                                }}
                            </button>
                            <button
                                class="btn"
                                on:click=move |_| on_cancel_prompt.run(())
                            >
                                "Cancel"
                            </button>
                        </div>
                    </div>
                </Show>
                <For
                    each=move || system_prompts.get()
                    key=|p| (p.id, p.name.clone())
                    children=move |p| {
                        let id = p.id;
                        let name = p.name.clone();
                        view! {
                            <div class="session">
                                <span class="session-name">{name}</span>
                                <span class="session-actions">
                                    <button
                                        class="icon-btn"
                                        title="Edit"
                                        on:click=move |_| on_edit_prompt.run(id)
                                    >
                                        "✎"
                                    </button>
                                    <button
                                        class="icon-btn"
                                        title="Delete"
                                        on:click=move |_| on_delete_prompt.run(id)
                                    >
                                        "✕"
                                    </button>
                                </span>
                            </div>
                        }
                    }
                />
                <Show when=move || system_prompts.get().is_empty() fallback=|| ()>
                    <p class="empty">"No prompts yet — create one."</p>
                </Show>
            </div>
        </aside>
    }
}
