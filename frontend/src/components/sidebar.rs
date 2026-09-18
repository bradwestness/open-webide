use leptos::prelude::*;
use openwebide_core::{ChatSession, Connection, Project, SystemPrompt, WorkspaceMode};
use web_sys::wasm_bindgen::JsCast;

#[component]
pub fn Sidebar(
    connections: ReadSignal<Vec<Connection>>,
    projects: ReadSignal<Vec<Project>>,
    sessions: ReadSignal<Vec<ChatSession>>,
    active_project: ReadSignal<Option<i64>>,
    active_session: ReadSignal<Option<i64>>,
    show_new_project: ReadSignal<bool>,
    np_name: ReadSignal<String>,
    set_np_name: WriteSignal<String>,
    np_mode: ReadSignal<WorkspaceMode>,
    set_np_mode: WriteSignal<WorkspaceMode>,
    np_path: ReadSignal<String>,
    set_np_path: WriteSignal<String>,
    on_new_project: Callback<()>,
    on_create_project: Callback<()>,
    on_cancel_new: Callback<()>,
    on_open_project: Callback<i64>,
    on_delete_project: Callback<i64>,
    on_select_session: Callback<i64>,
    on_new_session: Callback<()>,
    on_rename_session: Callback<i64>,
    on_delete_session: Callback<i64>,
    system_prompts: ReadSignal<Vec<SystemPrompt>>,
    show_prompt_form: ReadSignal<bool>,
    prompt_edit_id: ReadSignal<Option<i64>>,
    prompt_name: ReadSignal<String>,
    set_prompt_name: WriteSignal<String>,
    prompt_content: ReadSignal<String>,
    set_prompt_content: WriteSignal<String>,
    on_new_prompt: Callback<()>,
    on_edit_prompt: Callback<i64>,
    on_save_prompt: Callback<()>,
    on_cancel_prompt: Callback<()>,
    on_delete_prompt: Callback<i64>,
) -> impl IntoView {
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
        <aside class="sidebar">
            // -- projects ---------------------------------------------------
            <div class="sidebar-section">
                <div class="section-header">
                    <h2>"Projects"</h2>
                    <button
                        class="icon-btn"
                        title="New project"
                        on:click=move |_| on_new_project.run(())
                    >
                        "+"
                    </button>
                </div>
                <Show when=move || show_new_project.get() fallback=|| ()>
                    <div class="new-project-form">
                        <input
                            type="text"
                            class="form-input"
                            placeholder="Project name"
                            value=move || np_name.get()
                            on:input=move |e: web_sys::Event| {
                                if let Some(target) = e.target()
                                    && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                                {
                                    set_np_name.set(input.value());
                                }
                            }
                        />
                        <div class="mode-picker">
                            <label
                                class=move || {
                                    if np_mode.get() == WorkspaceMode::Remote {
                                        "mode-opt active".to_string()
                                    } else {
                                        "mode-opt".to_string()
                                    }
                                }
                            >
                                <input
                                    type="radio"
                                    name="ws-mode"
                                    checked=move || np_mode.get() == WorkspaceMode::Remote
                                    on:click=move |_| set_np_mode.set(WorkspaceMode::Remote)
                                />
                                "Remote"
                            </label>
                            <label
                                class=move || {
                                    if np_mode.get() == WorkspaceMode::Local {
                                        "mode-opt active".to_string()
                                    } else {
                                        "mode-opt".to_string()
                                    }
                                }
                            >
                                <input
                                    type="radio"
                                    name="ws-mode"
                                    checked=move || np_mode.get() == WorkspaceMode::Local
                                    on:click=move |_| set_np_mode.set(WorkspaceMode::Local)
                                />
                                "Local"
                            </label>
                        </div>
                        <Show when=move || np_mode.get() == WorkspaceMode::Remote fallback=|| ()>
                            <input
                                type="text"
                                class="form-input"
                                placeholder="Path relative to ~/source (e.g. repos/myproject)"
                                value=move || np_path.get()
                                on:input=move |e: web_sys::Event| {
                                    if let Some(target) = e.target()
                                        && let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>()
                                    {
                                        set_np_path.set(input.value());
                                    }
                                }
                            />
                        </Show>
                        <div class="form-actions">
                            <button
                                class="btn send"
                                on:click=move |_| on_create_project.run(())
                            >
                                "Open"
                            </button>
                            <button
                                class="btn"
                                on:click=move |_| on_cancel_new.run(())
                            >
                                "Cancel"
                            </button>
                        </div>
                    </div>
                </Show>
                <For
                    each=move || projects.get()
                    key=|p| p.id
                    children=move |p| {
                        let id = p.id;
                        let name = p.name.clone();
                        let mode = p.mode;
                        view! {
                            <div
                                class=move || {
                                    if active_project.get() == Some(id) {
                                        "project active".to_string()
                                    } else {
                                        "project".to_string()
                                    }
                                }
                                on:click=move |_| on_open_project.run(id)
                            >
                                <span class="project-name">{name}</span>
                                <span class="project-mode">{mode.as_str()}</span>
                                <span class="project-actions">
                                    <button
                                        class="icon-btn"
                                        title="Delete project"
                                        on:click=move |e: web_sys::MouseEvent| {
                                            e.stop_propagation();
                                            on_delete_project.run(id);
                                        }
                                    >
                                        "✕"
                                    </button>
                                </span>
                            </div>
                        }
                    }
                />
                <Show when=move || projects.get().is_empty() fallback=|| ()>
                    <p class="empty">"No projects yet — open a folder."</p>
                </Show>
            </div>

            // -- sessions (scoped to the active project) ---------------------
            <div class="sidebar-section">
                <div class="section-header">
                    <h2>"Sessions"</h2>
                    <button
                        class="icon-btn"
                        title="New session"
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
                        key=|s| s.id
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
                        <p class="empty">"No sessions yet — create one."</p>
                    </Show>
                </Show>
            </div>

            // -- connections --------------------------------------------------
            <div class="sidebar-section">
                <h2>"Connections"</h2>
                <For
                    each=move || connections.get()
                    key=|c| c.id
                    children=move |c| {
                        view! {
                            <div class="connection">
                                <span class="conn-name">{move || c.name.clone()}</span>
                                <span class="conn-kind">{c.kind.as_str()}</span>
                            </div>
                        }
                    }
                />
                <Show when=move || connections.get().is_empty() fallback=|| ()>
                    <p class="empty">"No connections yet — add one via the API."</p>
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
                    key=|p| p.id
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
