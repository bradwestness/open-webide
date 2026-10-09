use crate::{
    monitors::MonitorActions,
    state::{chat::ChatState, monitors::MonitorsState, projects::ProjectsState},
};
use leptos::prelude::*;
use openwebide_core::scheduled::MonitorCommand;
#[component]
pub fn ConversationMonitors() -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let projects = expect_context::<ProjectsState>();
    let state = use_context::<MonitorsState>();
    let actions = use_context::<MonitorActions>();
    let Some((state, actions)) = state.zip(actions) else {
        return ().into_any();
    };
    Effect::new(move |_| {
        chat.active_session.get();
        chat.streaming.get();
        actions.command.run(MonitorCommand::List {});
    });
    view! {
        <Show when=move || chat.active_session.get().is_some()>
            <super::ui::DisclosurePanel class="conversation-monitors" summary=move ||view!{<span>{move ||format!("Monitors ({})",state.entries.get().len())}</span>}>

            <section class="todo-notice" aria-label="Conversation monitors">
                <Show when=move || projects.active_project.get().and_then(|id|projects.project(id)).is_some_and(|project|project.mode==openwebide_core::WorkspaceMode::Local)>
                    <button class="btn sm" disabled=move ||state.busy.get() on:click=move |_|actions.authorize.run(())>"Authorize this folder's host"</button>
                </Show>
                <button class="btn sm ghost" disabled=move ||state.busy.get() on:click=move |_| actions.command.run(MonitorCommand::List {})>"Refresh"</button>
                <Show when=move ||state.error.get().is_some()><p role="alert">{move ||state.error.get()}</p></Show>
                <For each=move ||state.entries.get() key=|entry| (entry.id, entry.revision, entry.next_run, entry.last_run.clone()) children=move |entry| {
                    let status = entry.last_run.as_ref().map_or_else(|| "Pending".into(), |run| run.status.clone());
                    let when = entry.next_run.map(crate::scheduled::time);
                    let detail = entry.last_run.as_ref().map(|run| run.detail.clone());
                    view! { <div>
                        <span>{format!("Monitor #{}: {status}",entry.id)}</span>
                        {when.map(|when|view!{<span>{format!(" · next check {when}")}</span>})}
                        <p>{entry.draft.prompt.clone()}</p>
                        <Show when=move ||!entry.host_available><p class="form-hint">"Execution host unavailable; checks wait until it reconnects or the monitor expires."</p></Show>
                        {detail.map(|detail|view!{<p>{detail}</p>})}
                        <button class="btn sm stop" disabled=move ||state.busy.get() on:click=move |_|actions.command.run(MonitorCommand::Cancel{id:entry.id,revision:entry.revision})>"Cancel future checks"</button>
                    </div> }
                } />
            </section>
            </super::ui::DisclosurePanel>
        </Show>
    }.into_any()
}
