use crate::{
    project_skills::ProjectSkillActions,
    state::{projects::ProjectsState, skills::SkillsState},
};
use leptos::prelude::*;
use openwebide_core::{SkillCommand, SkillResource};
#[component]
pub fn Skills() -> impl IntoView {
    let state = expect_context::<SkillsState>();
    let actions = expect_context::<ProjectSkillActions>();
    let projects = expect_context::<ProjectsState>();
    let file = NodeRef::<leptos::html::Input>::new();
    let folder = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(input) = folder.get() {
            let _ = input.set_attribute("webkitdirectory", "");
        }
    });
    let import = move |event| {
        let input = event_target::<web_sys::HtmlInputElement>(&event);
        if let Some(files) = input.files() {
            actions.import.run(files);
        }
        input.set_value("");
    };
    view! {
        <Show when=move ||projects.active_project.get().is_some()>
            <section class="sidebar-section project-skills" aria-label="Project skills">
                <div class="section-header"><h2>"Skills"</h2><button class="icon-btn" title="New skill" aria-label="New skill" disabled=move ||state.busy.get() ||state.data.get().is_none() on:click=move |_|actions.edit.run(None)><super::ui::Icon name=super::ui::IconName::Plus/></button></div>
                <div class="session-filters"><super::ui::SegmentedControl options=vec![super::ui::SegmentOption::new("Enabled", true).disabled_when(Signal::derive(move ||state.data.get().is_none() ||state.busy.get())),super::ui::SegmentOption::new("Disabled", false).disabled_when(Signal::derive(move ||state.data.get().is_none() ||state.busy.get()))] value=Signal::derive(move ||state.data.with(|data|data.as_ref().is_none_or(|data|data.enabled))) on_change=Callback::new(move |enabled|actions.command.run(SkillCommand::SetEnabled {enabled}))/></div>
                <p class="form-hint">"Reusable workflows shared across this project’s sessions. Disabled skills remain saved."</p>
                <div class="form-actions">
                    <button class="btn ghost" disabled=move ||state.busy.get() ||state.data.get().is_none() on:click=move |_| { if let Some(file) = file.get() { file.click(); } }>"Import file or archive"</button>
                    <button class="btn ghost" disabled=move ||state.busy.get() ||state.data.get().is_none() on:click=move |_| { if let Some(folder) = folder.get() { folder.click(); } }>"Import folder"</button>
                </div>
                <input node_ref=file type="file" hidden accept=".md,.zip,.skill,.tar,.gz,.tgz" aria-label="Import skill file or archive" on:change=import/>
                <input node_ref=folder type="file" hidden multiple aria-label="Import skill folder" on:change=import/>
                <Show when=move ||state.loading.get() &&state.data.get().is_none()><p class="form-hint" role="status">"Loading skills…"</p></Show>
                <Show when=move ||state.busy.get()><p class="form-hint" role="status">"Working…"</p></Show>
                <Show when=move ||state.error.get().is_some()><p class="form-hint" role="alert">{move ||state.error.get().unwrap_or_default()}</p></Show>
                <Show when=move ||state.editing.get()>
                    <div class="memory-editor">
                        <super::ui::FormField label="Name"><input class="form-input" aria-label="Skill name" maxlength="64" placeholder="review-pull-request" disabled=move ||state.busy.get() prop:value=move ||state.draft.with(|draft|draft.name.clone()) on:input=move |event|state.draft.update(|draft|draft.name=event_target_value(&event))/></super::ui::FormField>
                        <super::ui::FormField label="Description and when to use"><textarea class="form-input" aria-label="Skill description" maxlength="1024" rows="3" disabled=move ||state.busy.get() prop:value=move ||state.draft.with(|draft|draft.description.clone()) on:input=move |event|state.draft.update(|draft|draft.description=event_target_value(&event))/></super::ui::FormField>
                        <super::ui::FormField label="Instructions"><textarea class="form-input" aria-label="Skill instructions" maxlength="32768" rows="8" disabled=move ||state.busy.get() prop:value=move ||state.draft.with(|draft|draft.instructions.clone()) on:input=move |event|state.draft.update(|draft|draft.instructions=event_target_value(&event))/></super::ui::FormField>
                        <super::ui::CheckboxField label="Enable this skill" checked=Signal::derive(move ||state.draft.with(|draft|draft.enabled)) disabled=Signal::derive(move ||state.busy.get()) on_change=Callback::new(move |enabled|state.draft.update(|draft|draft.enabled=enabled))/>

                        <p class="form-hint">"Supporting resources: references, scripts and assets. Imports open here for review; Save adds them to the database."</p>
                        <For each=move ||state.draft.with(|draft|(0..draft.resources.len()).collect::<Vec<_>>()) key=|index|*index children=move |index| view! {
                            <div class="memory-editor">
                                <super::ui::FormField label="Resource name"><input class="form-input" aria-label="Skill resource name" disabled=move ||state.busy.get() prop:value=move ||state.draft.with(|draft|draft.resources.get(index).map_or_else(String::new, |resource|resource.name.clone())) on:input=move |event|state.draft.update(|draft| { if let Some(resource)=draft.resources.get_mut(index) {resource.name=event_target_value(&event);} })/></super::ui::FormField>
                                <super::ui::FormField label="Resource content"><textarea class="form-input" aria-label="Skill resource content" rows="4" maxlength="32768" disabled=move ||state.busy.get() prop:value=move ||state.draft.with(|draft|draft.resources.get(index).map_or_else(String::new, |resource|resource.content.clone())) on:input=move |event|state.draft.update(|draft| { if let Some(resource)=draft.resources.get_mut(index) {resource.content=event_target_value(&event);} })/></super::ui::FormField>
                                <p class="form-hint">{move ||state.draft.with(|draft|if draft.resources.get(index).is_some_and(|r|r.binary) {"Binary asset (base64); preserved on export."} else {"Text resource"})}</p>
                                <button class="btn ghost" disabled=move ||state.busy.get() on:click=move |_|state.draft.update(|draft| { if index < draft.resources.len() { draft.resources.remove(index); } })>"Remove resource"</button>
                            </div>
                        }/>
                        <button class="btn ghost" disabled=move ||state.busy.get() ||state.draft.with(|draft|draft.resources.len()>=16) on:click=move |_|state.draft.update(|draft|draft.resources.push(SkillResource {name:String::new(),content:String::new(),binary:false}))>"Add resource"</button>
                        <div class="form-actions"><button class="btn" disabled=move ||state.busy.get() on:click=move |_|actions.save.run(())>"Save skill"</button><button class="btn ghost" disabled=move ||state.busy.get() on:click=move |_|state.editing.set(false)>"Cancel"</button></div>
                    </div>
                </Show>
                <For each=move ||state.data.get().map_or_else(Vec::new,|data|data.entries) key=|entry|(entry.id,entry.revision) children=move |entry| {
                    let edit=entry.clone();let export=entry.clone();let id=entry.id;let revision=entry.revision;let name=entry.draft.name.clone();let disabled= !entry.draft.enabled;
                    view! {<super::ui::DisclosurePanel class="memory-entry" summary=move ||view!{<span class="memory-title">{name.clone()}{disabled.then_some(" (disabled)")}</span>}>
                        <p class="memory-content">{entry.draft.description}</p><p class="memory-content">{entry.draft.instructions}</p>
                        <p class="form-hint">{format!("{} supporting resources",entry.draft.resources.len())}</p>
                        <div class="form-actions"><button class="btn ghost" disabled=move ||state.busy.get() on:click=move |_|actions.edit.run(Some(edit.clone()))>"Edit"</button><button class="btn ghost" disabled=move ||state.busy.get() on:click=move |_|actions.export.run(export.clone())>"Export ZIP"</button><button class="btn ghost" disabled=move ||state.busy.get() on:click=move |_|actions.command.run(SkillCommand::Delete {id,revision})>"Delete"</button></div>
                    </super::ui::DisclosurePanel>}
                }/>
                <Show when=move ||state.data.with(|data|data.as_ref().is_some_and(|data|data.entries.is_empty()))><p class="empty">"No project skills yet."</p></Show>
            </section>
        </Show>
    }
}
