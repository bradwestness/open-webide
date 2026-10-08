use crate::{
    project_memory::ProjectMemoryActions,
    state::{memories::MemoriesState, projects::ProjectsState},
};
use leptos::prelude::*;
use openwebide_core::MemoryCommand;
#[component]
pub fn Memories() -> impl IntoView {
    let state = expect_context::<MemoriesState>();
    let actions = expect_context::<ProjectMemoryActions>();
    let projects = expect_context::<ProjectsState>();
    view! {
        <Show when=move || projects.active_project.get().is_some()>
            <section class="sidebar-section project-memories" aria-label="Project memories">
                <div class="section-header"><h2>"Memories"</h2><button class="icon-btn" title="New memory" aria-label="New memory" disabled=move || state.busy.get() || state.data.get().is_none() on:click=move |_|actions.edit.run(None)><super::ui::Icon name=super::ui::IconName::Plus /></button></div>
                <div class="session-filters"><super::ui::SegmentedControl options=vec![super::ui::SegmentOption::new("Enabled", true).disabled_when(Signal::derive(move ||state.data.get().is_none() || state.busy.get())), super::ui::SegmentOption::new("Disabled", false).disabled_when(Signal::derive(move ||state.data.get().is_none() || state.busy.get()))] value=Signal::derive(move ||state.data.with(|data|data.as_ref().is_none_or(|data|data.enabled))) on_change=Callback::new(move |enabled|actions.command.run(MemoryCommand::SetEnabled {enabled})) /></div>
                <p class="form-hint">{move || if state.data.with(|data|data.as_ref().is_none_or(|data|data.enabled)) {"Shared across this project’s sessions."} else {"Off: stored memories stay available. New runs won’t use memory context or tools."}}</p>
                <Show when=move || state.loading.get() && state.data.get().is_none()><p class="form-hint" role="status">"Loading memories…"</p></Show>
                <Show when=move || state.error.get().is_some()><p class="form-hint" role="alert">{move ||state.error.get().unwrap_or_default()}</p></Show>
                <Show when=move || state.editing.get()>
                    <div class="memory-editor"><super::ui::FormField label="Title"><input class="form-input" aria-label="Memory title" placeholder="Name automatically" maxlength="120" disabled=move ||state.busy.get() prop:value=move ||state.title.get() on:input=move |event|{ let title = event_target_value(&event); state.auto_title.set(title.trim().is_empty()); state.title.set(title); }/></super::ui::FormField><super::ui::FormField label="Memory"><textarea class="form-input" aria-label="Memory content" maxlength="4000" rows="6" disabled=move ||state.busy.get() prop:value=move ||state.content.get() on:input=move |event|state.content.set(event_target_value(&event))></textarea></super::ui::FormField><div class="form-actions"><button class="btn" disabled=move ||state.busy.get() on:click=move |_|actions.save.run(())>"Save memory"</button><button class="btn ghost" disabled=move ||state.busy.get() on:click=move |_|state.editing.set(false)>"Cancel"</button></div></div>
                </Show>
                <For each=move ||state.data.get().map_or_else(Vec::new, |data|data.entries) key=|entry|(entry.id,entry.revision) children=move |entry| {
                    let edit = entry.clone();let id=entry.id;let revision=entry.revision;let title=entry.title;
                    view! {<super::ui::DisclosurePanel class="memory-entry" summary=move ||view!{<span class="memory-title">{title.clone()}</span>}><p class="memory-content">{entry.content}</p><div class="form-actions"><button class="btn ghost" disabled=move ||state.busy.get() on:click=move |_|actions.edit.run(Some(edit.clone()))>"Edit"</button><button class="btn ghost" disabled=move ||state.busy.get() on:click=move |_|actions.command.run(MemoryCommand::Delete {id,revision})>"Delete"</button></div></super::ui::DisclosurePanel>}
                }/>
                <Show when=move ||state.data.with(|data|data.as_ref().is_some_and(|data|data.entries.is_empty()))><p class="empty">"No project memories yet."</p></Show>
            </section>
        </Show>
    }
}
