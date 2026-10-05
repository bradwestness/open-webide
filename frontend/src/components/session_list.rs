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
            <super::ui::PanelSearchRow class="session-search-toolbar">
                <input id="session-search" class="form-input panel-search-input session-search" aria-label="Search session names and messages" type="search" maxlength="256" placeholder="Search sessions…" prop:value=move || state.query.get() on:input=move |event|state.query.set(event_target_value(&event)) />
                <button class="icon-btn" title="New chat" aria-label="New chat" disabled=move || chat.creating_session.get() || chat.streaming.get() on:click=move |_| on_new.run(())><super::ui::Icon name=super::ui::IconName::Plus /></button>
            </super::ui::PanelSearchRow>
            <div class="session-filters"><super::ui::SegmentedControl options=vec![super::ui::SegmentOption::new("Active", false), super::ui::SegmentOption::new("Archived", true)] value=Signal::derive(move || state.archived.get()) on_change=Callback::new(move |archived| state.archived.set(archived)) /></div>
            <Show when=move || state.searching.get()><p class="form-hint" role="status">"Searching…"</p></Show>
            <Show when=move || state.search_error.get().is_some()><p class="form-hint" role="alert">{move || state.search_error.get().unwrap_or_default()}</p></Show>
            <For each=move || state.visible.get() key=|session| (session.id,session.name.clone(),session.pinned,session.archived) children=move |session| {
                let id=session.id;let pinned=session.pinned;let archived=session.archived;
                let title=session.name.clone();
                view! {
                    <div class=move || format!("session{}",if chat.active_session.get()==Some(id){" active"}else{""}) data-session-id=id data-context-menu="">
                        <button class="btn ghost session-name" title=title on:click=move |_|on_select.run(id)>{pinned.then(|| view! { <crate::components::ui::Icon name=crate::components::ui::IconName::Pin /> })}<span class="session-label">{session.name}</span></button>
                        <super::dropdown::ActionMenu aria_label="Session actions">
                            <button role="menuitem" class="ui-dropdown-item recent-item icon-btn session-pin" title=if pinned {"Unpin"}else{"Pin"} aria-pressed=pinned.to_string() disabled=move || state.busy.with(|busy|busy.contains(&id)) on:click=move |_|actions.preferences.run((id,SessionPreferences{pinned:Some(!pinned),archived:None}))><crate::components::ui::Icon name=crate::components::ui::IconName::Pin /><span>{if pinned { "Unpin" } else { "Pin" }}</span></button>
                            <button role="menuitem" class="ui-dropdown-item recent-item icon-btn session-archive" title=if archived {"Restore session"}else{"Archive session"} disabled=move || state.busy.with(|busy|busy.contains(&id)) on:click=move |_|actions.preferences.run((id,SessionPreferences{pinned:None,archived:Some(!archived)}))><crate::components::ui::Icon name=if archived {crate::components::ui::IconName::ArchiveRestore}else{crate::components::ui::IconName::Archive} /><span>{if archived { "Restore session" } else { "Archive session" }}</span></button>
                            <button role="menuitem" class="ui-dropdown-item recent-item icon-btn session-export" title="Export Markdown" disabled=move || state.exporting.with(|busy|busy.contains(&id)) on:click=move |_|actions.export.run(id)><crate::components::ui::Icon name=crate::components::ui::IconName::Download /><span>"Export Markdown"</span></button>
                            <button role="menuitem" class="ui-dropdown-item recent-item icon-btn" title="Rename" on:click=move |_|on_rename.run(id)><crate::components::ui::Icon name=crate::components::ui::IconName::Pencil /><span>"Rename"</span></button>
                            <button role="menuitem" class="ui-dropdown-item recent-item icon-btn" title="Delete" on:click=move |_|on_delete.run(id)><crate::components::ui::Icon name=crate::components::ui::IconName::X /><span>"Delete"</span></button>
                        </super::dropdown::ActionMenu>
                    </div>
                }
            }/>
            <Show when=move || state.visible.with(Vec::is_empty) && !state.searching.get() && state.search_error.get().is_none()><p class="empty">{move || if !state.query.get().trim().is_empty(){"No matching sessions."}else if state.archived.get(){"No archived sessions."}else{"No sessions yet — start chatting to create one."}}</p></Show>
        </div>
    }
}
