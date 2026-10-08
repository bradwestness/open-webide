use crate::state::settings::{ConfigurationSection, SettingsState};
use leptos::prelude::*;
use web_sys::wasm_bindgen::JsCast;

/// App-level server and prompt management using the existing shared settings actions.
#[component]
pub fn Configuration(
    on_new_connection: Callback<()>,
    on_edit_connection: Callback<i64>,
    on_cancel_connection: Callback<()>,
    on_delete_connection: Callback<i64>,
    on_new_prompt: Callback<()>,
    on_edit_prompt: Callback<i64>,
    on_save_prompt: Callback<()>,
    on_cancel_prompt: Callback<()>,
    on_delete_prompt: Callback<i64>,
) -> impl IntoView {
    let settings = expect_context::<SettingsState>();

    let connections = settings.connections.read_only();
    let system_prompts = settings.system_prompts.read_only();
    let show_prompt_form = settings.show_prompt_form.read_only();
    let prompt_edit_id = settings.prompt_edit_id.read_only();
    let prompt_name = settings.prompt_name.read_only();
    let set_prompt_name = settings.prompt_name.write_only();
    let prompt_content = settings.prompt_content.read_only();
    let set_prompt_content = settings.prompt_content.write_only();

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

    let auth = expect_context::<crate::state::auth::AuthState>();
    Effect::new(move |previous: Option<u64>| {
        let epoch = auth.generation.get();
        if previous.is_some_and(|previous| previous != epoch) {
            settings.configuration.set(None);
            settings.show_conn_form.set(false);
            settings.show_prompt_form.set(false);
        }
        epoch
    });
    view! {
        <Show when=move || settings.configuration.get().is_some()>
            <super::modal::Modal title=Signal::derive(|| "Configuration".to_string()) on_close=Callback::new(move |()| settings.configuration.set(None)) size=super::ui::DialogSize::Wide>
                <super::ui::DialogBody class="app-configuration">
                    <super::ui::SegmentedControl options=vec![super::ui::SegmentOption::new("Servers", ConfigurationSection::Servers),super::ui::SegmentOption::new("System prompts", ConfigurationSection::SystemPrompts)] value=Signal::derive(move || settings.configuration.get().unwrap_or(ConfigurationSection::Servers)) on_change=Callback::new(move |section| settings.configuration.set(Some(section))) />
            // -- connections --------------------------------------------------
            <div class="sidebar-section" hidden=move || settings.configuration.get() != Some(crate::state::settings::ConfigurationSection::Servers)>
                <div class="section-header">
                    <h2>"Servers"</h2>
                    <button
                        class="icon-btn"
                        title="New server" aria-label="New server"
                        on:click=move |_| on_new_connection.run(())
                    >
                        <super::ui::Icon name=super::ui::IconName::Plus />
                    </button>
                </div>
                <For
                    each=move || connections.get()
                    key=|c| (c.id, c.name.clone(), c.kind)
                    children=move |c| {
                        let id = c.id;
                        let name = c.name.clone();
                        let kind = c.kind;
                        view! {
                            <div class="connection" data-context-menu="">
                                <span class="conn-name" title=name.clone()>{name.clone()}</span>
                                <span class="conn-kind" title=kind.display_name()>{kind.display_name()}</span>
                                <super::dropdown::ActionMenu aria_label="Server actions">
                                    <button role="menuitem"
                                        class="ui-dropdown-item recent-item icon-btn"
                                        title="Edit"
                                        on:click=move |_| on_edit_connection.run(id)
                                    >
                                        <crate::components::ui::Icon name=crate::components::ui::IconName::Pencil />
                                    <span>"Edit"</span></button>
                                    <button role="menuitem"
                                        class="ui-dropdown-item recent-item icon-btn"
                                        title="Delete"
                                        on:click=move |_| on_delete_connection.run(id)
                                    >
                                        <crate::components::ui::Icon name=crate::components::ui::IconName::X />
                                    <span>"Delete"</span></button>
                                </super::dropdown::ActionMenu>
                            </div>
                        }
                    }
                />
                <Show when=move || connections.get().is_empty() fallback=|| ()>
                    <p class="empty">"No connections yet — add one."</p>
                </Show>
            </div>

            // -- system prompts ----------------------------------------------
            <div class="sidebar-section" hidden=move || settings.configuration.get() != Some(crate::state::settings::ConfigurationSection::SystemPrompts)>
                <div class="section-header">
                    <h2>"System prompts"</h2>
                    <button
                        class="icon-btn"
                        title="New prompt" aria-label="New prompt"
                        on:click=move |_| on_new_prompt.run(())
                    >
                        <super::ui::Icon name=super::ui::IconName::Plus />
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
                            <div class="session" data-context-menu="">
                                <span class="session-name">{name}</span>
                                <super::dropdown::ActionMenu aria_label="Prompt actions">
                                    <button role="menuitem"
                                        class="ui-dropdown-item recent-item icon-btn"
                                        title="Edit"
                                        on:click=move |_| on_edit_prompt.run(id)
                                    >
                                        <crate::components::ui::Icon name=crate::components::ui::IconName::Pencil />
                                    <span>"Edit"</span></button>
                                    <button role="menuitem"
                                        class="ui-dropdown-item recent-item icon-btn"
                                        title="Delete"
                                        on:click=move |_| on_delete_prompt.run(id)
                                    >
                                        <crate::components::ui::Icon name=crate::components::ui::IconName::X />
                                    <span>"Delete"</span></button>
                                </super::dropdown::ActionMenu>
                            </div>
                        }
                    }
                />
                <Show when=move || system_prompts.get().is_empty() fallback=|| ()>
                    <p class="empty">"No prompts yet — create one."</p>
                </Show>
            </div>

                </super::ui::DialogBody>
            </super::modal::Modal>
        </Show>
        <Show when=move || settings.show_conn_form.get()><super::model_wizard::ModelSetupWizard on_close=on_cancel_connection /></Show>
    }
}
