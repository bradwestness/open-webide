use crate::{
    state::{chat::ChatState, sessions::SessionsState},
    state_actions::sessions::SessionActions,
};
use leptos::prelude::*;
use openwebide_core::SessionPreferences;

#[component]
pub fn SessionList(
    on_select: Callback<i64>,
    on_new: Callback<()>,
    on_rename: Callback<i64>,
    on_delete: Callback<i64>,
) -> impl IntoView {
    let chat = expect_context::<ChatState>();
    let state = expect_context::<SessionsState>();
    let actions = SessionActions::from_context();
    view! {
        <div class="sidebar-section session-list">
            <div class="section-header"><h2>"Sessions"</h2><button class="icon-btn" title="New chat" disabled=move || chat.creating_session.get() || chat.streaming.get() on:click=move |_| on_new.run(())>"+"</button></div>
            <input id="session-search" class="form-input session-search" aria-label="Search session names and messages" type="search" maxlength="256" placeholder="Search sessions…" prop:value=move || state.query.get() on:input=move |event|state.query.set(event_target_value(&event)) />
            <div class="session-filters"><button class="btn ghost" aria-pressed=move || state.archived.get().to_string() on:click=move |_| state.archived.update(|archived|*archived = !*archived)>{move || if state.archived.get() {"Archived sessions"} else {"Active sessions"}}</button></div>
            <Show when=move || state.searching.get()><p class="form-hint" role="status">"Searching…"</p></Show>
            <Show when=move || state.search_error.get().is_some()><p class="form-hint" role="alert">{move || state.search_error.get().unwrap_or_default()}</p></Show>
            <For each=move || state.visible.get() key=|session| (session.id,session.name.clone(),session.pinned,session.archived) children=move |session| {
                let id=session.id;let pinned=session.pinned;let archived=session.archived;
                let title=session.name.clone();
                view! {
                    <div class=move || format!("session{}",if chat.active_session.get()==Some(id){" active"}else{""}) data-session-id=id>
                        <button class="btn ghost session-name" title=title on:click=move |_|on_select.run(id)>{pinned.then(|| view! { <crate::components::ui::Icon name=crate::components::ui::IconName::Pin /> })}{session.name}</button>
                        <span class="session-actions">
                            <button class="icon-btn session-pin" title=if pinned {"Unpin"}else{"Pin"} aria-pressed=pinned.to_string() disabled=move || state.busy.with(|busy|busy.contains(&id)) on:click=move |_|actions.preferences.run((id,SessionPreferences{pinned:Some(!pinned),archived:None}))><crate::components::ui::Icon name=crate::components::ui::IconName::Pin /></button>
                            <button class="icon-btn session-archive" title=if archived {"Restore session"}else{"Archive session"} disabled=move || state.busy.with(|busy|busy.contains(&id)) on:click=move |_|actions.preferences.run((id,SessionPreferences{pinned:None,archived:Some(!archived)}))><crate::components::ui::Icon name=if archived {crate::components::ui::IconName::ArchiveRestore}else{crate::components::ui::IconName::Archive} /></button>
                            <button class="icon-btn session-export" title="Export Markdown" disabled=move || state.exporting.with(|busy|busy.contains(&id)) on:click=move |_|actions.export.run(id)><crate::components::ui::Icon name=crate::components::ui::IconName::Download /></button>
                            <button class="icon-btn" title="Rename" on:click=move |_|on_rename.run(id)><crate::components::ui::Icon name=crate::components::ui::IconName::Pencil /></button>
                            <button class="icon-btn" title="Delete" on:click=move |_|on_delete.run(id)><crate::components::ui::Icon name=crate::components::ui::IconName::X /></button>
                        </span>
                    </div>
                }
            }/>
            <Show when=move || state.visible.with(Vec::is_empty) && !state.searching.get() && state.search_error.get().is_none()><p class="empty">{move || if !state.query.get().trim().is_empty(){"No matching sessions."}else if state.archived.get(){"No archived sessions."}else{"No sessions yet — start chatting to create one."}}</p></Show>
        </div>
    }
}
