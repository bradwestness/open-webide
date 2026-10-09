use super::ui::{Button, FormField, FormSection, InlineActions, TextInput};
use crate::{
    project_plugins::ProjectPluginActions,
    state::{plugins::PluginsState, projects::ProjectsState},
};
use leptos::prelude::*;

#[component]
pub fn Plugins() -> impl IntoView {
    let state = expect_context::<PluginsState>();
    let actions = expect_context::<ProjectPluginActions>();
    let projects = expect_context::<ProjectsState>();
    view! {
        <FormSection title="Plugins" description="Install a package on this project's execution host. Installed packages do not yet activate agent skills.">
            <Show when=move || projects.active_project.get().is_some() fallback=||view!{<p class="form-hint">"Open a project to install a plugin on its host."</p>}>
                <FormField label="Repository URL"><TextInput label="Plugin repository URL" value=state.repository.read_only() on_change=Callback::new(move |value|state.repository.set(value)) maxlength=2048 disabled=state.busy.read_only()/></FormField>
                <FormField label="Commit"><TextInput label="Plugin commit" value=state.commit.read_only() on_change=Callback::new(move |value|state.commit.set(value)) maxlength=64 disabled=state.busy.read_only()/></FormField>
                <FormField label="Package directory"><TextInput label="Plugin package directory" value=state.path.read_only() on_change=Callback::new(move |value|state.path.set(value)) maxlength=512 disabled=state.busy.read_only()/></FormField>
                <p class="form-hint">"Use a full commit ID and the directory containing plugin.json. Use . for the repository root."</p>
                <InlineActions><Button disabled=state.busy.read_only() on_click=Callback::new(move |_|actions.install.run(()))>"Install on project host"</Button></InlineActions>
            </Show>
            <InlineActions><Button disabled=state.busy.read_only() on_click=Callback::new(move |_|actions.refresh.run(()))>"Refresh installations"</Button></InlineActions>
            <Show when=move ||state.busy.get()><p class="form-hint" role="status">"Working…"</p></Show>
            <Show when=move ||state.error.get().is_some()><p class="error" role="alert">{move ||state.error.get().unwrap_or_default()}</p></Show>
            <For each=move ||state.installations.get() key=|entry|(entry.prepared.source.repository.clone(),entry.prepared.source.path.clone(),entry.revision) children=move |entry| {
                view! {<super::ui::DisclosurePanel summary=move ||format!("{} {}",entry.prepared.manifest.display_name,entry.prepared.manifest.version)>
                    <p class="form-hint">{entry.prepared.manifest.description}</p>
                    <p class="form-hint">{format!("Installed on {} host(s). Agent activation is not enabled.",entry.hosts.len())}</p>
                    <p class="form-hint">{format!("{} · {} · {}",entry.prepared.source.repository,entry.prepared.source.path,entry.prepared.source.commit)}</p>
                </super::ui::DisclosurePanel>}
            }/>
        </FormSection>
    }
}
